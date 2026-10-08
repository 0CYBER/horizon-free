//! The End: End portal frames filled with eyes of ender, End portal and End
//! gateway travel, the obsidian arrival platform, the exit portal and the End
//! sky. The ender dragon is not implemented: the fight counts as already won,
//! so the exit portal is active and the first gateway exists.
use super::{Dim, Game};
use horizon_client::{
    Result, environment::Environment, items::ItemStack, mesher::Vertex, motion::PlayerMotion,
    vanillagen::VanillaTerrain,
};
use horizon_core::{
    geometry::Vec3,
    random::{LegacyRandom, RandomSource},
};
use std::{collections::HashMap, sync::Arc};

/// ServerLevel.END_SPAWN_POINT.
const END_SPAWN: [i32; 3] = [100, 50, 0];
/// EnderDragonFight gateway ring radius and height.
const GATEWAY_RADIUS: f64 = 96.0;
const GATEWAY_Y: i32 = 75;
/// File (in the End region folder) keeping End gateway exits.
const GATEWAY_FILE: &str = "horizon-end-gateways.json";

/// A gateway teleport waiting for its destination chunks.
#[derive(Clone, Copy, Debug)]
enum GatewayJob {
    /// TheEndGatewayBlockEntity.findOrCreateValidTeleportPos from this gateway.
    Find { gateway: [i32; 3] },
    /// A known exit (exact, or findExitPosition around it).
    Known { exit: [i32; 3], exact: bool },
}

#[derive(Default)]
pub(super) struct EndState {
    /// Gateway position -> (exit, exact teleport); loaded lazily from the save.
    exits: Option<HashMap<[i32; 3], ([i32; 3], bool)>>,
    job: Option<GatewayJob>,
    /// Ticks until the player can use a gateway again (TheEndGatewayBlockEntity cooldown).
    gateway_cooldown: u32,
    /// End portal and gateway blocks per loaded chunk (true = gateway), for rendering.
    portals: HashMap<(i32, i32), Vec<([i32; 3], bool)>>,
    /// Chunks left to scan for portal blocks in the current round.
    scan: Vec<(i32, i32)>,
    /// The Overworld generator, for stronghold positions (ServerLevel.findNearestMapStructure).
    locator: std::sync::Mutex<Option<Arc<VanillaTerrain>>>,
    /// Thrown eyes of ender in flight.
    eyes: Vec<EyeOfEnder>,
    flash: EndFlash,
}

/// EndFlashState: a purple flash in the End sky every 600 ticks that also
/// lights the world (the lightmap's sky factor).
#[derive(Default)]
struct EndFlash {
    seed: Option<i64>,
    offset: i32,
    duration: i32,
    intensity: f32,
    old_intensity: f32,
    x_angle: f32,
    y_angle: f32,
    /// Ticks until the delayed weather.end_flash sound.
    sound_in: Option<u32>,
}
impl EndFlash {
    fn tick(&mut self, clock: i64) {
        let seed = clock.div_euclid(600);
        if self.seed != Some(seed) {
            let mut random = LegacyRandom::new(seed);
            random.next_float();
            let between = |r: &mut LegacyRandom, lo: i32, hi: i32| {
                r.next_int_bound(hi - lo + 1).unwrap_or(0) + lo
            };
            self.offset = between(&mut random, 0, 200);
            self.duration = between(&mut random, 100, 380.min(600 - self.offset));
            self.x_angle = random.next_float() * 70.0 - 60.0;
            self.y_angle = random.next_float() * 360.0 - 180.0;
            self.seed = Some(seed);
        }
        self.old_intensity = self.intensity;
        let t = clock.rem_euclid(600) as i32;
        self.intensity = if t >= self.offset && t <= self.offset + self.duration {
            ((t - self.offset) as f32 * std::f32::consts::PI / self.duration as f32).sin()
        } else {
            0.0
        };
        if self.intensity > 0.0 && self.old_intensity <= 0.0 {
            self.sound_in = Some(30);
        }
    }
    fn intensity(&self, partial: f32) -> f32 {
        self.old_intensity + (self.intensity - self.old_intensity) * partial
    }
}
impl EndState {
    pub(super) fn set_locator(&self, terrain: Option<Arc<VanillaTerrain>>) {
        *self.locator.lock().unwrap() = terrain;
    }
}

/// The EyeOfEnder entity.
struct EyeOfEnder {
    position: glam::DVec3,
    previous: glam::DVec3,
    velocity: glam::DVec3,
    target: glam::DVec3,
    life: u32,
    survive: bool,
}
impl EyeOfEnder {
    /// EyeOfEnder.updateDeltaMovement.
    fn update_delta(old: glam::DVec3, position: glam::DVec3, target: glam::DVec3) -> glam::DVec3 {
        let horizontal = glam::DVec3::new(target.x - position.x, 0.0, target.z - position.z);
        let length = horizontal.length();
        let old_horizontal = old.x.hypot(old.z);
        let mut wanted_speed = old_horizontal + 0.0025 * (length - old_horizontal);
        let mut y = old.y;
        if length < 1.0 {
            wanted_speed *= 0.8;
            y *= 0.8;
        }
        let wanted_y = if position.y - old.y < target.y {
            1.0
        } else {
            -1.0
        };
        horizontal * (wanted_speed / length)
            + glam::DVec3::new(0.0, y + (wanted_y - y) * 0.015, 0.0)
    }
}

/// Chunks scanned for portal blocks per tick.
const SCAN_PER_TICK: usize = 6;
/// Radius (chunks) around the player in which portal blocks are rendered.
const PORTAL_RADIUS: i32 = 10;

impl Game<'_> {
    /// The environment attributes of the current dimension.
    pub(super) fn dim_environment(&self) -> &Environment {
        match self.dim {
            Dim::Overworld => &self.res.environment,
            Dim::Nether => &self.res.nether_environment,
            Dim::End => &self.res.end_environment,
        }
    }

    /// SkyRenderer.buildEndSky: six faces of end_sky.png tiled 16 times,
    /// tinted 0x282828. The atlas does not wrap, so each face is 16×16 quads.
    pub(super) fn end_sky_geometry(&self) -> Vec<Vertex> {
        let mut out = Vec::with_capacity(6 * 256 * 4);
        let Some(r) = self.res.atlas.sprites.get("environment/end_sky").copied() else {
            return out;
        };
        use std::f32::consts::FRAC_PI_2;
        let faces = [
            glam::Mat4::IDENTITY,
            glam::Mat4::from_rotation_x(FRAC_PI_2),
            glam::Mat4::from_rotation_x(-FRAC_PI_2),
            glam::Mat4::from_rotation_x(std::f32::consts::PI),
            glam::Mat4::from_rotation_z(FRAC_PI_2),
            glam::Mat4::from_rotation_z(-FRAC_PI_2),
        ];
        let step = 200.0 / 16.0;
        for pose in faces {
            for a in 0..16 {
                for b in 0..16 {
                    let (x0, z0) = (-100.0 + step * a as f32, -100.0 + step * b as f32);
                    let (x1, z1) = (x0 + step, z0 + step);
                    let corners = [
                        (x0, z0, [r.u0, r.v0]),
                        (x0, z1, [r.u0, r.v1]),
                        (x1, z1, [r.u1, r.v1]),
                        (x1, z0, [r.u1, r.v0]),
                    ];
                    for (x, z, uv) in corners {
                        let p = pose.transform_point3(glam::Vec3::new(x, -100.0, z));
                        out.push(Vertex {
                            pos: p.to_array(),
                            uv,
                            layer: 2,
                            color: 0xFF28_2828,
                            light: 0,
                        });
                    }
                }
            }
        }
        // SkyRenderer.renderEndFlash: the end_flash sprite at height 100, scale 60.
        let intensity = self.end.flash.intensity(self.clock.fraction() as f32);
        if intensity > 1e-5
            && let Some(r) = self
                .res
                .atlas
                .sprites
                .get("environment/celestial/end_flash")
                .copied()
        {
            let flash = &self.end.flash;
            let pose = glam::Mat4::from_rotation_y((180.0 - flash.y_angle).to_radians())
                * glam::Mat4::from_rotation_x((-90.0 - flash.x_angle).to_radians());
            let level = (intensity.clamp(0.0, 1.0) * 255.0) as u32;
            let color = level << 24 | level << 16 | level << 8 | level;
            let corners = [
                (-1.0, -1.0, [r.u0, r.v0]),
                (1.0, -1.0, [r.u1, r.v0]),
                (1.0, 1.0, [r.u1, r.v1]),
                (-1.0, 1.0, [r.u0, r.v1]),
            ];
            for (x, z, uv) in corners {
                let p = pose.transform_point3(glam::Vec3::new(x * 60.0, 100.0, z * 60.0));
                out.push(Vertex {
                    pos: p.to_array(),
                    uv,
                    layer: 2,
                    color,
                    light: 0,
                });
            }
        }
        out
    }

    /// LightmapRenderStateExtractor: End flashes add to the sky light factor.
    pub(super) fn end_flash_sky(&self, sky: &mut horizon_client::environment::Sky, partial: f64) {
        if self.dim == Dim::End {
            sky.sky_light_factor += self.end.flash.intensity(partial as f32);
        }
    }

    fn block_name(&self, p: [i32; 3]) -> std::sync::Arc<str> {
        self.res
            .facts
            .get(self.world.block(p[0], p[1], p[2]))
            .name
            .clone()
    }

    /// EnderEyeItem.useOn: fill an empty End portal frame, then light the
    /// portal when the ring of twelve filled frames is complete.
    pub(super) fn use_ender_eye(&mut self, stack: &ItemStack) -> Result<bool> {
        if &*stack.item != "minecraft:ender_eye" {
            return Ok(false);
        }
        let registry = self.res.assets.registry;
        let target = self.pick().and_then(|hit| {
            let state = registry.state(self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]))?;
            (state.name == "minecraft:end_portal_frame").then(|| (hit, state.clone()))
        });
        let Some((hit, state)) = target else {
            // EnderEyeItem.use: throw the eye towards the nearest stronghold.
            self.throw_ender_eye();
            return Ok(true);
        };
        if state.properties.get("eye").map(String::as_str) == Some("true") {
            // Both useOn and use pass on a filled frame.
            return Ok(true);
        }
        let mut filled = state;
        filled.properties.insert("eye".into(), "true".into());
        let id = registry.resolve(&filled)?;
        self.level_set(hit.pos, id)?;
        // Block.pushEntitiesUp: the eye raises the frame's top from 13/16 to a full block.
        if let Some(p) = self.player.as_mut() {
            let top = f64::from(hit.pos[1]) + 1.0;
            let w = f64::from(p.dimensions.width) / 2.0;
            let over = |c: f64, b: i32| c + w > f64::from(b) && c - w < f64::from(b) + 1.0;
            if over(p.position.x, hit.pos[0])
                && over(p.position.z, hit.pos[2])
                && p.position.y >= top - 0.25
                && p.position.y < top
            {
                p.position.y = top;
            }
        }
        self.consume_held();
        self.swing_arm();
        let center = hit.pos.map(|v| f64::from(v) + 0.5);
        // LevelEvent 1503: END_PORTAL_FRAME_FILL.
        self.audio.play(
            "minecraft:block.end_portal_frame.fill",
            Some(center),
            1.0,
            1.0,
        );
        if let Some(c) = self.portal_ring_center(hit.pos) {
            for dz in -1..=1 {
                for dx in -1..=1 {
                    self.set_state_named(
                        [c[0] + dx, c[1], c[2] + dz],
                        "minecraft:end_portal",
                        &[],
                    )?;
                }
            }
            self.rescan_portals_around(c, 1);
            // Global LevelEvent 1038: END_PORTAL_SPAWN.
            self.audio
                .play("minecraft:block.end_portal.spawn", None, 1.0, 1.0);
        }
        Ok(true)
    }

    /// EndPortalFrameBlock.getOrCreatePortalShape matched around `frame`: a 5×5
    /// ring (corners free) of filled frames, all facing the centre or all
    /// facing away from it. Returns the centre of the 3×3 opening.
    fn portal_ring_center(&self, frame: [i32; 3]) -> Option<[i32; 3]> {
        let registry = self.res.assets.registry;
        // (offset from the centre, facing when the frames face the centre)
        let mut ring = Vec::with_capacity(12);
        for i in -1..=1 {
            ring.push(([i, -2], "south"));
            ring.push(([i, 2], "north"));
            ring.push(([-2, i], "east"));
            ring.push(([2, i], "west"));
        }
        let opposite = |f: &str| match f {
            "south" => "north",
            "north" => "south",
            "east" => "west",
            _ => "east",
        };
        for &([ox, oz], _) in &ring {
            let c = [frame[0] - ox, frame[1], frame[2] - oz];
            for inward in [true, false] {
                let ok = ring.iter().all(|&([dx, dz], facing)| {
                    let id = self.world.block(c[0] + dx, c[1], c[2] + dz);
                    let Some(s) = registry.state(id) else {
                        return false;
                    };
                    let want = if inward { facing } else { opposite(facing) };
                    s.name == "minecraft:end_portal_frame"
                        && s.properties.get("eye").map(String::as_str) == Some("true")
                        && s.properties.get("facing").map(String::as_str) == Some(want)
                });
                if ok {
                    return Some(c);
                }
            }
        }
        None
    }

    /// End portals, End gateways and the exit portal, once per tick.
    pub(super) fn tick_end(&mut self) -> Result<()> {
        self.end.gateway_cooldown = self.end.gateway_cooldown.saturating_sub(1);
        self.scan_portals();
        self.tick_eyes();
        if self.dim == Dim::End {
            self.end.flash.tick(self.world.game_time);
            if let Some(n) = self.end.flash.sound_in.as_mut() {
                *n = n.saturating_sub(1);
                if *n == 0 {
                    self.end.flash.sound_in = None;
                    self.audio
                        .play("minecraft:weather.end_flash", None, 1.0, 1.0);
                }
            }
        }
        if self.dim == Dim::End {
            self.ensure_exit_portal()?;
        }
        let Some(p) = self.player.as_ref() else {
            return Ok(());
        };
        if self.travelling.is_some() || p.health.dead_or_dying() {
            return Ok(());
        }
        let w = f64::from(p.dimensions.width) / 2.0;
        let lo = [p.position.x - w, p.position.y, p.position.z - w];
        let hi = [
            p.position.x + w,
            p.position.y + f64::from(p.dimensions.height),
            p.position.z + w,
        ];
        let mut portal = false;
        let mut gateway = None;
        for x in lo[0].floor() as i32..=hi[0].floor() as i32 {
            for y in lo[1].floor() as i32..=hi[1].floor() as i32 {
                for z in lo[2].floor() as i32..=hi[2].floor() as i32 {
                    let name = self.block_name([x, y, z]);
                    // EndPortalBlock entity-inside shape: y 6/16..12/16.
                    if &*name == "minecraft:end_portal"
                        && lo[1] < f64::from(y) + 0.75
                        && hi[1] > f64::from(y) + 0.375
                    {
                        portal = true;
                    } else if &*name == "minecraft:end_gateway" && self.dim == Dim::End {
                        gateway = Some([x, y, z]);
                    }
                }
            }
        }
        if portal {
            if self.portal_cooldown == 0 {
                let target = if self.dim == Dim::End {
                    Dim::Overworld
                } else {
                    Dim::End
                };
                self.end_portal_travel = true;
                let result = self.enter_dimension(target);
                self.end_portal_travel = false;
                result?;
                // Player.getDimensionChangingDelay.
                self.portal_cooldown = 10;
            } else {
                // Entity.setAsInsidePortal while cooling down restarts the cooldown.
                self.portal_cooldown = 10;
            }
        } else if let Some(g) = gateway
            && self.end.gateway_cooldown == 0
        {
            self.enter_gateway(g)?;
        }
        Ok(())
    }

    /// EndPortalBlock.getPortalDestination for the pending enter_dimension.
    pub(super) fn end_destination(&self, target: Dim) -> Option<Vec3> {
        if !self.end_portal_travel {
            return None;
        }
        if target == Dim::End {
            // Vec3.atBottomCenterOf(END_SPAWN_POINT), one block lower for players.
            return Some(Vec3::new(
                f64::from(END_SPAWN[0]) + 0.5,
                f64::from(END_SPAWN[1] - 1),
                f64::from(END_SPAWN[2]) + 0.5,
            ));
        }
        // Leaving the End: the respawn position (the world spawn here).
        let s = self.level.spawn.unwrap_or([0.0, 64.0, 0.0]);
        Some(Vec3::new(s[0].floor() + 0.5, s[1], s[2].floor() + 0.5))
    }

    /// Arrival after End portal or gateway travel, once the destination is loaded.
    pub(super) fn place_after_end_travel(
        &mut self,
        mut player: PlayerMotion,
        target: Vec3,
    ) -> Result<()> {
        self.end_travel = false;
        // Player.getDimensionChangingDelay (enter_dimension uses the Nether portal value).
        self.portal_cooldown = 10;
        if player.health.dead_or_dying() {
            // Respawning out of the End: a fresh player at the spawn.
            player = PlayerMotion::new(self.motion, target)?;
            self.eye = 1.62;
            self.previous_eye = 1.62;
            self.clock.reset();
        }
        let mut position = target;
        if let Some(job) = self.end.job.take() {
            position = self.resolve_gateway(job, target)?;
        } else if self.dim == Dim::End {
            // EndPlatformFeature.createEndPlatform below the spawn point.
            let origin = [END_SPAWN[0], END_SPAWN[1] - 1, END_SPAWN[2]];
            for dz in -2..=2 {
                for dx in -2..=2 {
                    for dy in -1..3 {
                        let p = [origin[0] + dx, origin[1] + dy, origin[2] + dz];
                        let name = if dy == -1 {
                            "minecraft:obsidian"
                        } else {
                            "minecraft:air"
                        };
                        if &*self.block_name(p) != name {
                            self.set_state_named(p, name, &[])?;
                        }
                    }
                }
            }
            // Direction.WEST.toYRot().
            self.yaw = 90.0;
            player.yaw = 90.0;
            player.head_yaw = 90.0;
        } else {
            // ServerPlayer.findRespawnPositionAndUseSpawnBlock: onto the spawn column.
            let y = self
                .land_column(target.x.floor() as i32, target.z.floor() as i32)
                .map_or(target.y, f64::from);
            position = Vec3::new(target.x, y, target.z);
        }
        player.position = position;
        player.velocity = Vec3::ZERO;
        player.fall_distance = 0.0;
        self.previous_position = player.position;
        self.player = Some(player);
        self.apply_equipment();
        let pitch = self.world.random.next_float() * 0.4 + 0.8;
        self.audio
            .play("minecraft:block.portal.travel", None, 0.25, pitch);
        Ok(())
    }

    /// Dying in the End respawns in the Overworld.
    pub(super) fn respawn_out_of_end(&mut self) -> Result<()> {
        if self.dim != Dim::End || self.player.is_none() {
            return Ok(());
        }
        self.end_portal_travel = true;
        let result = self.enter_dimension(Dim::Overworld);
        self.end_portal_travel = false;
        result
    }

    /// EnderDragonFight without a dragon: once the centre of the main island
    /// is loaded, place the active exit portal (EndPodiumFeature) and the
    /// first gateway (spawnNewGateway) if the portal is not there yet.
    fn ensure_exit_portal(&mut self) -> Result<()> {
        if self.travelling.is_some()
            || !(-1..=0).all(|cz| (-1..=0).all(|cx| self.streaming.is_lit(cx, cz)))
        {
            return Ok(());
        }
        let gateway = self.first_gateway();
        if (0..256).any(|y| &*self.block_name([0, y, 0]) == "minecraft:bedrock") {
            // The podium exists; the gateway may have been out of range when it was placed.
            if self.world.loaded(gateway[0], gateway[2])
                && &*self.block_name(gateway) != "minecraft:end_gateway"
                && &*self.block_name([gateway[0], gateway[1] + 2, gateway[2]])
                    != "minecraft:bedrock"
            {
                self.place_gateway(gateway)?;
            }
            return Ok(());
        }
        // getHeightmapPos(MOTION_BLOCKING_NO_LEAVES, podium location).below()
        let mut y = (0..256)
            .rev()
            .find(|&y| {
                let f = self.res.facts.get(self.world.block(0, y, 0));
                (f.full_collision || f.fluid != horizon_client::blocks::Fluid::None)
                    && !f.name.ends_with("_leaves")
            })
            .unwrap_or(0);
        while &*self.block_name([0, y, 0]) == "minecraft:bedrock" && y > 63 {
            y -= 1;
        }
        let origin = [0, y.max(1), 0];
        self.place_podium(origin)?;
        if self.world.loaded(gateway[0], gateway[2]) {
            self.place_gateway(gateway)?;
        }
        Ok(())
    }

    /// The first gateway of the shuffled ring (EnderDragonFight constructor
    /// and spawnNewGateway).
    fn first_gateway(&self) -> [i32; 3] {
        let mut gateways: Vec<i32> = (0..20).collect();
        let mut random = LegacyRandom::new(self.level.seed);
        for i in (2..=gateways.len()).rev() {
            let j = random.next_int_bound(i as i32).unwrap_or(0) as usize;
            gateways.swap(i - 1, j);
        }
        let index = f64::from(gateways[gateways.len() - 1]);
        let angle = 2.0 * (-std::f64::consts::PI + std::f64::consts::PI / 20.0 * index);
        [
            (GATEWAY_RADIUS * angle.cos()).floor() as i32,
            GATEWAY_Y,
            (GATEWAY_RADIUS * angle.sin()).floor() as i32,
        ]
    }

    /// EndPodiumFeature(active).place at `origin` (the portal level).
    fn place_podium(&mut self, origin: [i32; 3]) -> Result<()> {
        for y in origin[1] - 1..=origin[1] + 32 {
            for z in origin[2] - 4..=origin[2] + 4 {
                for x in origin[0] - 4..=origin[0] + 4 {
                    let d =
                        (x - origin[0]).pow(2) + (y - origin[1]).pow(2) + (z - origin[2]).pow(2);
                    let d = f64::from(d);
                    let inside_rim = d < 2.5 * 2.5;
                    if !(inside_rim || d < 3.5 * 3.5) {
                        continue;
                    }
                    let name = if y < origin[1] {
                        if inside_rim {
                            "minecraft:bedrock"
                        } else {
                            "minecraft:end_stone"
                        }
                    } else if y > origin[1] {
                        "minecraft:air"
                    } else if !inside_rim {
                        "minecraft:bedrock"
                    } else {
                        "minecraft:end_portal"
                    };
                    self.set_state_named([x, y, z], name, &[])?;
                }
            }
        }
        for dy in 0..4 {
            self.set_state_named(
                [origin[0], origin[1] + dy, origin[2]],
                "minecraft:bedrock",
                &[],
            )?;
        }
        let registry = self.res.assets.registry;
        for (facing, d) in [
            ("north", [0, -1]),
            ("south", [0, 1]),
            ("west", [-1, 0]),
            ("east", [1, 0]),
        ] {
            let Some(default) = registry.default_state("minecraft:wall_torch") else {
                continue;
            };
            let mut state = registry.state(default).cloned().ok_or("wall torch")?;
            state.properties.insert("facing".into(), facing.into());
            let id = registry.resolve(&state)?;
            self.level_set([origin[0] + d[0], origin[1] + 2, origin[2] + d[1]], id)?;
        }
        self.rescan_portals_around(origin, 4);
        Ok(())
    }

    /// EndGatewayFeature.place: the gateway block in a bedrock cross.
    fn place_gateway(&mut self, origin: [i32; 3]) -> Result<()> {
        for y in -2i32..=2 {
            for z in -1..=1 {
                for x in -1..=1 {
                    let (same_x, same_y, same_z) = (x == 0, y == 0, z == 0);
                    let end = y.abs() == 2;
                    let name = if same_x && same_y && same_z {
                        "minecraft:end_gateway"
                    } else if same_y {
                        "minecraft:air"
                    } else if end && same_x && same_z || (same_x || same_z) && !end {
                        "minecraft:bedrock"
                    } else {
                        "minecraft:air"
                    };
                    self.set_state_named([origin[0] + x, origin[1] + y, origin[2] + z], name, &[])?;
                }
            }
        }
        self.rescan_portals_around(origin, 0);
        Ok(())
    }

    fn gateway_exits(&mut self) -> &mut HashMap<[i32; 3], ([i32; 3], bool)> {
        if self.end.exits.is_none() {
            let path = horizon_client::save::dimension_dir(&self.world_root, "minecraft:the_end")
                .join(GATEWAY_FILE);
            let parsed: Option<Vec<([i32; 3], [i32; 3], bool)>> = std::fs::read(&path)
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok());
            self.end.exits = Some(
                parsed
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(g, e, x)| (g, (e, x)))
                    .collect(),
            );
        }
        self.end.exits.as_mut().expect("loaded exits")
    }

    fn set_gateway_exit(&mut self, gateway: [i32; 3], exit: [i32; 3], exact: bool) -> Result<()> {
        self.gateway_exits().insert(gateway, (exit, exact));
        let list: Vec<_> = self
            .gateway_exits()
            .iter()
            .map(|(g, (e, x))| (*g, *e, *x))
            .collect();
        let dir = horizon_client::save::dimension_dir(&self.world_root, "minecraft:the_end");
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join(GATEWAY_FILE), serde_json::to_vec(&list)?)?;
        Ok(())
    }

    /// EndGatewayBlock.entityInside and TheEndGatewayBlockEntity.getPortalPosition:
    /// travel within the End, waiting for the destination chunks to load.
    fn enter_gateway(&mut self, gateway: [i32; 3]) -> Result<()> {
        // TheEndGatewayBlockEntity.triggerCooldown: 40 ticks.
        self.end.gateway_cooldown = 40;
        let known = self.gateway_exits().get(&gateway).copied();
        let job = match known {
            Some((exit, exact)) => GatewayJob::Known { exit, exact },
            // Worldgen return gateways (end_gateway_return) lead exactly to the End spawn.
            None if f64::from(gateway[0]).hypot(f64::from(gateway[2])) > 512.0 => {
                GatewayJob::Known {
                    exit: END_SPAWN,
                    exact: true,
                }
            }
            None => GatewayJob::Find { gateway },
        };
        let destination = match job {
            GatewayJob::Known { exit, .. } => Vec3::new(
                f64::from(exit[0]) + 0.5,
                f64::from(exit[1]),
                f64::from(exit[2]) + 0.5,
            ),
            GatewayJob::Find { gateway } => {
                let d = glam::DVec2::new(f64::from(gateway[0]), f64::from(gateway[2]))
                    .normalize_or_zero()
                    * 1024.0;
                Vec3::new(d.x, 75.0, d.y)
            }
        };
        let Some(player) = self.player.take() else {
            return Ok(());
        };
        self.end.job = Some(job);
        self.end_travel = true;
        self.travelling = Some((player, destination));
        Ok(())
    }

    fn chunk_empty(&self, x: f64, z: f64) -> Option<bool> {
        let chunk = self
            .world
            .chunks
            .get(&((x / 16.0).floor() as i32, (z / 16.0).floor() as i32))?;
        Some(chunk.sections.iter().all(|s| s.is_uniform(0)))
    }

    /// TheEndGatewayBlockEntity.findTallestBlock.
    fn tallest_block(&self, around: [i32; 3], dist: i32, allow_bedrock: bool) -> [i32; 3] {
        let mut tallest: Option<[i32; 3]> = None;
        for xd in -dist..=dist {
            for zd in -dist..=dist {
                if xd == 0 && zd == 0 && !allow_bedrock {
                    continue;
                }
                let floor = tallest.map_or(0, |t| t[1]);
                let mut y = 255;
                while y > floor {
                    let p = [around[0] + xd, y, around[2] + zd];
                    let f = self.res.facts.get(self.world.block(p[0], p[1], p[2]));
                    if f.full_collision && (allow_bedrock || &*f.name != "minecraft:bedrock") {
                        tallest = Some(p);
                        break;
                    }
                    y -= 1;
                }
            }
        }
        tallest.unwrap_or(around)
    }

    /// The teleport position of a gateway job, creating the exit gateway (and
    /// an island when the chunk has no landing spot) for new destinations.
    fn resolve_gateway(&mut self, job: GatewayJob, target: Vec3) -> Result<Vec3> {
        let bottom_center = |p: [i32; 3]| {
            Vec3::new(
                f64::from(p[0]) + 0.5,
                f64::from(p[1]),
                f64::from(p[2]) + 0.5,
            )
        };
        let find_exit = |game: &Self, exit: [i32; 3]| {
            let p = game.tallest_block([exit[0], exit[1] + 2, exit[2]], 5, false);
            [p[0], p[1] + 1, p[2]]
        };
        match job {
            GatewayJob::Known { exit, exact } => Ok(bottom_center(if exact {
                exit
            } else {
                find_exit(self, exit)
            })),
            GatewayJob::Find { gateway } => {
                // findExitPortalXZPosTentative over the loaded chunks.
                let dir = glam::DVec2::new(f64::from(gateway[0]), f64::from(gateway[2]))
                    .normalize_or_zero();
                let mut pos = glam::DVec2::new(target.x, target.z);
                let mut limit = 16;
                while self.chunk_empty(pos.x, pos.y) == Some(false) && limit > 0 {
                    limit -= 1;
                    pos -= dir * 16.0;
                }
                let mut limit = 16;
                while self.chunk_empty(pos.x, pos.y) == Some(true) && limit > 0 {
                    limit -= 1;
                    pos += dir * 16.0;
                }
                let (cx, cz) = ((pos.x / 16.0).floor() as i32, (pos.y / 16.0).floor() as i32);
                // findValidSpawnInChunk: end stone with two non-full blocks above, closest to 0,0,0.
                let mut closest: Option<([i32; 3], f64)> = None;
                for y in 30..256 {
                    for z in cz * 16..cz * 16 + 16 {
                        for x in cx * 16..cx * 16 + 16 {
                            if &*self.block_name([x, y, z]) != "minecraft:end_stone"
                                || self
                                    .res
                                    .facts
                                    .get(self.world.block(x, y + 1, z))
                                    .full_collision
                                || self
                                    .res
                                    .facts
                                    .get(self.world.block(x, y + 2, z))
                                    .full_collision
                            {
                                continue;
                            }
                            let d = (f64::from(x) + 0.5).powi(2)
                                + (f64::from(y) + 0.5).powi(2)
                                + (f64::from(z) + 0.5).powi(2);
                            if closest.is_none_or(|c| d < c.1) {
                                closest = Some(([x, y, z], d));
                            }
                        }
                    }
                }
                let spot = match closest {
                    Some((p, _)) => p,
                    None => {
                        let p = [
                            (pos.x + 0.5).floor() as i32,
                            75,
                            (pos.y + 0.5).floor() as i32,
                        ];
                        self.place_end_island(p)?;
                        p
                    }
                };
                let tallest = self.tallest_block(spot, 16, true);
                let exit = [tallest[0], tallest[1] + 10, tallest[2]];
                self.place_gateway(exit)?;
                self.set_gateway_exit(exit, gateway, false)?;
                self.set_gateway_exit(gateway, exit, false)?;
                Ok(bottom_center(find_exit(self, exit)))
            }
        }
    }

    /// EndIslandFeature with RandomSource.create(pos.asLong()).
    fn place_end_island(&mut self, origin: [i32; 3]) -> Result<()> {
        let packed = ((i64::from(origin[0]) & 0x3FF_FFFF) << 38)
            | ((i64::from(origin[2]) & 0x3FF_FFFF) << 12)
            | (i64::from(origin[1]) & 0xFFF);
        let mut random = LegacyRandom::new(packed);
        let mut size = random.next_int_bound(3).unwrap_or(0) as f32 + 4.0;
        let mut y = 0;
        while size > 0.5 {
            for x in (-size).floor() as i32..=size.ceil() as i32 {
                for z in (-size).floor() as i32..=size.ceil() as i32 {
                    if (x * x + z * z) as f32 <= (size + 1.0) * (size + 1.0) {
                        self.set_state_named(
                            [origin[0] + x, origin[1] + y, origin[2] + z],
                            "minecraft:end_stone",
                            &[],
                        )?;
                    }
                }
            }
            size -= random.next_int_bound(2).unwrap_or(0) as f32 + 0.5;
            y -= 1;
        }
        Ok(())
    }

    /// Scan a few loaded chunks per tick for End portal and gateway blocks.
    fn scan_portals(&mut self) {
        let Some(p) = self.player.as_ref() else {
            return;
        };
        let (pcx, pcz) = (
            (p.position.x.floor() as i32) >> 4,
            (p.position.z.floor() as i32) >> 4,
        );
        let chunks = &self.world.chunks;
        self.end.portals.retain(|k, _| chunks.contains_key(k));
        if self.end.scan.is_empty() {
            let mut queue: Vec<(i32, i32)> = chunks
                .keys()
                .copied()
                .filter(|&(x, z)| {
                    (x - pcx).abs() <= PORTAL_RADIUS && (z - pcz).abs() <= PORTAL_RADIUS
                })
                .collect();
            // Nearest last: they are popped first.
            queue.sort_by_key(|&(x, z)| std::cmp::Reverse((x - pcx).pow(2) + (z - pcz).pow(2)));
            self.end.scan = queue;
        }
        for _ in 0..SCAN_PER_TICK {
            let Some((cx, cz)) = self.end.scan.pop() else {
                break;
            };
            self.scan_portal_chunk(cx, cz);
        }
    }

    fn scan_portal_chunk(&mut self, cx: i32, cz: i32) {
        let registry = self.res.assets.registry;
        let (Some(portal), Some(gateway)) = (
            registry.default_state("minecraft:end_portal"),
            registry.default_state("minecraft:end_gateway"),
        ) else {
            return;
        };
        let (portal, gateway) = (portal.0 as u16, gateway.0 as u16);
        let Some(chunk) = self.world.chunks.get(&(cx, cz)) else {
            self.end.portals.remove(&(cx, cz));
            return;
        };
        let mut found = Vec::new();
        for (s, section) in chunk.sections.iter().enumerate() {
            let horizon_client::chunk::Blocks::Full(blocks) = &section.blocks else {
                continue;
            };
            for (i, &b) in blocks.iter().enumerate() {
                if b == portal || b == gateway {
                    let y = horizon_client::chunk::MIN_Y + s as i32 * 16 + (i >> 8) as i32;
                    found.push((
                        [
                            cx * 16 + (i & 15) as i32,
                            y,
                            cz * 16 + ((i >> 4) & 15) as i32,
                        ],
                        b == gateway,
                    ));
                }
            }
        }
        if found.is_empty() {
            self.end.portals.remove(&(cx, cz));
        } else {
            self.end.portals.insert((cx, cz), found);
        }
    }

    fn rescan_portals_around(&mut self, p: [i32; 3], r: i32) {
        for cz in (p[2] - r) >> 4..=(p[2] + r) >> 4 {
            for cx in (p[0] - r) >> 4..=(p[0] + r) >> 4 {
                self.scan_portal_chunk(cx, cz);
            }
        }
    }

    /// End portal and gateway surfaces (TheEndPortalRenderer: the starfield
    /// approximated by end_portal.png in world space, unanimated) and the End
    /// crystals on the spikes, as camera-relative entity geometry.
    pub(super) fn end_geometry(&self, camera: glam::DVec3, out: &mut Vec<Vertex>) {
        let atlas = &self.res.atlas;
        if let Some(r) = atlas.sprites.get("entity/end_portal/end_portal").copied() {
            let full = 15 * 16 | (15 * 16) << 8;
            // Each block shows a quarter of the texture so neighbours join up.
            let quarter = |a: i32, b: i32| {
                let (fa, fb) = (a.rem_euclid(4) as f32 / 4.0, b.rem_euclid(4) as f32 / 4.0);
                let (du, dv) = (r.u1 - r.u0, r.v1 - r.v0);
                (
                    r.u0 + du * fa,
                    r.u0 + du * (fa + 0.25),
                    r.v0 + dv * fb,
                    r.v0 + dv * (fb + 0.25),
                )
            };
            for blocks in self.end.portals.values() {
                for &(p, gateway) in blocks {
                    let base = glam::DVec3::new(f64::from(p[0]), f64::from(p[1]), f64::from(p[2]))
                        - camera;
                    if base.length_squared() > 160.0 * 160.0 {
                        continue;
                    }
                    let b = base.as_vec3();
                    // Portal: top face at 12/16 and bottom at 6/16; gateway: the whole cube.
                    let (y0, y1) = if gateway { (0.0, 1.0) } else { (0.375, 0.75) };
                    let color = if gateway { 0xFFE0_E0A0 } else { 0xFFD0_E070 };
                    let mut quad =
                        |corners: [glam::Vec3; 4], (u0, u1, v0, v1): (f32, f32, f32, f32)| {
                            let uv = [[u0, v0], [u1, v0], [u1, v1], [u0, v1]];
                            for (c, uv) in corners.into_iter().zip(uv) {
                                out.push(Vertex {
                                    pos: (b + c).to_array(),
                                    uv,
                                    layer: 0,
                                    color,
                                    light: full,
                                });
                            }
                        };
                    let v = glam::Vec3::new;
                    let open = |d: [i32; 3]| {
                        let n = [p[0] + d[0], p[1] + d[1], p[2] + d[2]];
                        !self
                            .res
                            .facts
                            .get(self.world.block(n[0], n[1], n[2]))
                            .solid_render
                    };
                    let uv_xz = quarter(p[0], p[2]);
                    if !gateway || open([0, 1, 0]) {
                        quad(
                            [
                                v(0.0, y1, 0.0),
                                v(1.0, y1, 0.0),
                                v(1.0, y1, 1.0),
                                v(0.0, y1, 1.0),
                            ],
                            uv_xz,
                        );
                    }
                    if !gateway || open([0, -1, 0]) {
                        quad(
                            [
                                v(0.0, y0, 0.0),
                                v(1.0, y0, 0.0),
                                v(1.0, y0, 1.0),
                                v(0.0, y0, 1.0),
                            ],
                            uv_xz,
                        );
                    }
                    if gateway {
                        let uv_x = quarter(p[2], p[1]);
                        let uv_z = quarter(p[0], p[1]);
                        if open([-1, 0, 0]) {
                            quad(
                                [
                                    v(0.0, 0.0, 0.0),
                                    v(0.0, 0.0, 1.0),
                                    v(0.0, 1.0, 1.0),
                                    v(0.0, 1.0, 0.0),
                                ],
                                uv_x,
                            );
                        }
                        if open([1, 0, 0]) {
                            quad(
                                [
                                    v(1.0, 0.0, 0.0),
                                    v(1.0, 0.0, 1.0),
                                    v(1.0, 1.0, 1.0),
                                    v(1.0, 1.0, 0.0),
                                ],
                                uv_x,
                            );
                        }
                        if open([0, 0, -1]) {
                            quad(
                                [
                                    v(0.0, 0.0, 0.0),
                                    v(1.0, 0.0, 0.0),
                                    v(1.0, 1.0, 0.0),
                                    v(0.0, 1.0, 0.0),
                                ],
                                uv_z,
                            );
                        }
                        if open([0, 0, 1]) {
                            quad(
                                [
                                    v(0.0, 0.0, 1.0),
                                    v(1.0, 0.0, 1.0),
                                    v(1.0, 1.0, 1.0),
                                    v(0.0, 1.0, 1.0),
                                ],
                                uv_z,
                            );
                        }
                    }
                }
            }
        }
        if self.dim == Dim::End {
            self.end_crystal_geometry(camera, out);
        }
        self.eye_geometry(camera, out);
    }

    /// EndCrystalRenderer and EndCrystalModel for the crystals EndSpikeFeature
    /// spawns on the spikes (render only; crystals are not entities yet).
    fn end_crystal_geometry(&self, camera: glam::DVec3, out: &mut Vec<Vertex>) {
        let Some(r) = self
            .res
            .atlas
            .sprites
            .get("entity/end_crystal/end_crystal")
            .copied()
        else {
            return;
        };
        let age = self.ticks as f32 + self.clock.fraction() as f32;
        let sin45 = std::f32::consts::FRAC_PI_4.sin();
        let axis = glam::Vec3::new(sin45, 0.0, sin45).normalize();
        let tilt = glam::Quat::from_axis_angle(axis, std::f32::consts::FRAC_PI_3);
        let spin = glam::Quat::from_rotation_y((age * 3.0).to_radians());
        // getY: the bob, in blocks.
        let hh = (age * 0.2).sin() / 2.0 + 0.5;
        let bob = (hh * hh + hh) * 0.4 - 1.4;
        // A 8×8×8 model box at texture offset (u, v) of the 64×32 texture.
        let uv = |u: f32, v: f32, w: f32, h: f32| {
            let su = |x: f32| r.u0 + (r.u1 - r.u0) * x / 64.0;
            let sv = |y: f32| r.v0 + (r.v1 - r.v0) * y / 32.0;
            (su(u), su(u + w), sv(v), sv(v + h))
        };
        for spike in horizon_world::worldgen::end_spike::spikes_for_seed(self.level.seed) {
            let feet = glam::DVec3::new(
                f64::from(spike.center_x) + 0.5,
                f64::from(spike.height + 1),
                f64::from(spike.center_z) + 0.5,
            );
            let rel = feet - camera;
            if rel.length_squared() > 256.0 * 256.0 {
                continue;
            }
            let light = self.world.light(
                feet.x.floor() as i32,
                feet.y.floor() as i32,
                feet.z.floor() as i32,
            );
            let packed = u32::from(light >> 4) * 16 | (u32::from(light & 15) * 16) << 8;
            // poseStack.scale(2) then translate(0, -0.5, 0); outer_glass at y 24 px + bob.
            let center = rel.as_vec3() + glam::Vec3::new(0.0, 2.0 * (24.0 / 16.0 - 0.5) + bob, 0.0);
            let outer = spin * tilt;
            let inner = tilt * spin;
            let parts = [
                (outer, 1.0, 0.0),
                (outer * inner, 0.875, 0.0),
                (outer * inner * inner, 0.875 * 0.765_625, 32.0),
            ];
            for (rotation, scale, u) in parts {
                // Half extent: 4 px, times the renderer's scale of 2.
                let h = 0.25 * 2.0 * scale;
                let corner = |x: f32, y: f32, z: f32| {
                    center + rotation * glam::Vec3::new(x * h, y * h, z * h)
                };
                let faces: [([f32; 3], [f32; 3], [f32; 3], (f32, f32, f32, f32)); 6] = [
                    (
                        [-1.0, 1.0, -1.0],
                        [1.0, 0.0, 0.0],
                        [0.0, 0.0, 1.0],
                        uv(u + 8.0, 0.0, 8.0, 8.0),
                    ),
                    (
                        [-1.0, -1.0, -1.0],
                        [1.0, 0.0, 0.0],
                        [0.0, 0.0, 1.0],
                        uv(u + 16.0, 0.0, 8.0, 8.0),
                    ),
                    (
                        [-1.0, -1.0, -1.0],
                        [0.0, 0.0, 1.0],
                        [0.0, 1.0, 0.0],
                        uv(u, 8.0, 8.0, 8.0),
                    ),
                    (
                        [-1.0, -1.0, -1.0],
                        [1.0, 0.0, 0.0],
                        [0.0, 1.0, 0.0],
                        uv(u + 8.0, 8.0, 8.0, 8.0),
                    ),
                    (
                        [1.0, -1.0, -1.0],
                        [0.0, 0.0, 1.0],
                        [0.0, 1.0, 0.0],
                        uv(u + 16.0, 8.0, 8.0, 8.0),
                    ),
                    (
                        [-1.0, -1.0, 1.0],
                        [1.0, 0.0, 0.0],
                        [0.0, 1.0, 0.0],
                        uv(u + 24.0, 8.0, 8.0, 8.0),
                    ),
                ];
                for (o, a, b, (u0, u1, v0, v1)) in faces {
                    let p = |s: f32, t: f32| {
                        corner(
                            o[0] + 2.0 * (a[0] * s + b[0] * t),
                            o[1] + 2.0 * (a[1] * s + b[1] * t),
                            o[2] + 2.0 * (a[2] * s + b[2] * t),
                        )
                    };
                    let quad = [
                        (p(0.0, 0.0), [u0, v1]),
                        (p(1.0, 0.0), [u1, v1]),
                        (p(1.0, 1.0), [u1, v0]),
                        (p(0.0, 1.0), [u0, v0]),
                    ];
                    for (pos, uv) in quad {
                        out.push(Vertex {
                            pos: pos.to_array(),
                            uv,
                            layer: 0,
                            color: 0xFFFF_FFFF,
                            light: packed,
                        });
                    }
                }
            }
        }
    }

    /// EnderEyeItem.use: findNearestMapStructure over the stronghold rings,
    /// then an EyeOfEnder signalled towards it. Without a stronghold (other
    /// dimensions) nothing is thrown.
    fn throw_ender_eye(&mut self) {
        let rings = self
            .end
            .locator
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|t| t.ring_positions("minecraft:strongholds"));
        let (Some(rings), Some(p)) = (rings, self.player.as_ref()) else {
            return;
        };
        let block = [
            p.position.x.floor(),
            p.position.y.floor(),
            p.position.z.floor(),
        ];
        // ChunkGenerator.getNearestGeneratedStructure: the ring chunk closest to
        // its (8, 32, 8) block, located at the chunk's corner.
        let Some(nearest) = rings.iter().min_by(|a, b| {
            let d = |c: &horizon_core::position::ChunkPos| {
                (f64::from(c.x * 16 + 8) - block[0]).powi(2)
                    + (32.0 - block[1]).powi(2)
                    + (f64::from(c.z * 16 + 8) - block[2]).powi(2)
            };
            d(a).total_cmp(&d(b))
        }) else {
            return;
        };
        let target = glam::DVec3::new(f64::from(nearest.x * 16), 0.0, f64::from(nearest.z * 16));
        let start = glam::DVec3::new(
            p.position.x,
            p.position.y + f64::from(p.dimensions.height) * 0.5,
            p.position.z,
        );
        // EyeOfEnder.signalTo.
        let delta = target - start;
        let horizontal = delta.x.hypot(delta.z);
        let target = if horizontal > 12.0 {
            start
                + glam::DVec3::new(
                    delta.x / horizontal * 12.0,
                    8.0,
                    delta.z / horizontal * 12.0,
                )
        } else {
            target
        };
        let survive = self.world.random.next_int_bound(5).unwrap_or(0) > 0;
        self.end.eyes.push(EyeOfEnder {
            position: start,
            previous: start,
            velocity: glam::DVec3::ZERO,
            target,
            life: 0,
            survive,
        });
        let r = self.world.random.next_float();
        let pitch = 0.33 + r * (0.5 - 0.33);
        let at = [p.position.x, p.position.y, p.position.z];
        self.audio
            .play("minecraft:entity.ender_eye.launch", Some(at), 1.0, pitch);
        self.consume_held();
        self.swing_arm();
    }

    /// EyeOfEnder.tick: fly, then drop (4 in 5) or shatter after 80 ticks.
    fn tick_eyes(&mut self) {
        let mut eyes = std::mem::take(&mut self.end.eyes);
        eyes.retain_mut(|eye| {
            eye.previous = eye.position;
            let next = eye.position + eye.velocity;
            eye.velocity = EyeOfEnder::update_delta(eye.velocity, next, eye.target);
            eye.position = next;
            eye.life += 1;
            if eye.life <= 80 {
                return true;
            }
            let at = eye.position.to_array();
            self.audio
                .play("minecraft:entity.ender_eye.death", Some(at), 1.0, 1.0);
            if eye.survive {
                // new ItemEntity(level, x, y, z, item): random spread, 0.2 up.
                let vx = self.world.random.next_double() * 0.2 - 0.1;
                let vz = self.world.random.next_double() * 0.2 - 0.1;
                let stack = ItemStack {
                    item: "minecraft:ender_eye".into(),
                    count: 1,
                    damage: 0,
                    extra: None,
                };
                self.spawn_item(stack, at, [vx, 0.2, vz], 0);
            }
            false
        });
        self.end.eyes = eyes;
    }

    /// ThrownItemRenderer: the ender eye item facing the camera.
    fn eye_geometry(&self, camera: glam::DVec3, out: &mut Vec<Vertex>) {
        let Some(r) = self.res.atlas.sprites.get("item/ender_eye").copied() else {
            return;
        };
        let alpha = self.clock.fraction();
        let (yaw, pitch) = (self.yaw.to_radians(), self.pitch.to_radians());
        let right = glam::Vec3::new(-yaw.cos(), 0.0, -yaw.sin()) * 0.25;
        let up = glam::Vec3::new(
            -yaw.sin() * pitch.sin(),
            pitch.cos(),
            yaw.cos() * pitch.sin(),
        ) * 0.25;
        let full = 15 * 16 | (15 * 16) << 8;
        for eye in &self.end.eyes {
            let p = eye.previous + (eye.position - eye.previous) * alpha - camera;
            let c = p.as_vec3();
            let corners = [
                (c - right - up, [r.u0, r.v1]),
                (c + right - up, [r.u1, r.v1]),
                (c + right + up, [r.u1, r.v0]),
                (c - right + up, [r.u0, r.v0]),
            ];
            for (pos, uv) in corners {
                out.push(Vertex {
                    pos: pos.to_array(),
                    uv,
                    layer: 0,
                    color: 0xFFFF_FFFF,
                    light: full,
                });
            }
        }
    }
}
