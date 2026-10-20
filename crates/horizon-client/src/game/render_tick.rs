//! Per-tick client visuals: the particle engine, `ClientLevel.animateTick`,
//! campfire smoke and rain splashes (`ClientLevel.tickWeatherEffects`).
use super::{Dim, Game};
use horizon_client::fog::{FogData, FogInputs, FogType, fog_type, update_rain_fog};
use horizon_client::{
    animate_tick::{AnimateContext, animate_tick, campfire_particles, marker_particle_target},
    biome::Biomes,
    blocks::{BlockFacts, Fluid},
    chunk::{HEIGHT, MIN_Y},
    particles::{ParticleLevel, ParticleOptions, ParticleType},
    world::World,
};
use horizon_client::{
    hud_overlays,
    screen_effects::{ScreenInputs, ScreenQuad, lightmap_brightness},
};
use horizon_core::random::{LegacyRandom, RandomSource};

/// The first motion-blocking block at or below `from` (Heightmap MOTION_BLOCKING), plus one.
fn motion_blocking(world: &World, facts: &BlockFacts, x: i32, z: i32, from: i32) -> i32 {
    let mut y = from.min(MIN_Y + HEIGHT - 1);
    while y > MIN_Y {
        let f = facts.get(world.block(x, y, z));
        if f.full_collision || f.fluid != Fluid::None || f.name.contains("leaves") {
            return y + 1;
        }
        y -= 1;
    }
    MIN_Y
}
/// `Biome.getPrecipitationAt` is rain (not snow) at this position.
fn rains_at(world: &World, biomes: &Biomes, x: i32, y: i32, z: i32) -> bool {
    let Some(chunk) = world.chunks.get(&(x >> 4, z >> 4)) else {
        return false;
    };
    let b = biomes.get(chunk.biome(x, y, z));
    if !b.precipitation {
        return false;
    }
    let t = if y > 80 {
        b.temperature - (y - 80) as f32 * 0.05 / 40.0
    } else {
        b.temperature
    };
    t >= 0.15
}

impl Game<'_> {
    pub(super) fn forward(&self) -> [f64; 3] {
        let (yaw, pitch) = (
            f64::from(self.yaw).to_radians(),
            f64::from(self.pitch).to_radians(),
        );
        [
            -yaw.sin() * pitch.cos(),
            -pitch.sin(),
            yaw.cos() * pitch.cos(),
        ]
    }
    /// `FogRenderer.setupFog` for the frame's camera.
    pub(super) fn frame_fog(
        &self,
        camera: glam::DVec3,
        sky: &horizon_client::environment::Sky,
        biome: u16,
    ) -> FogData {
        let forward = self.forward();
        let at = [camera.x, camera.y, camera.z];
        let fog_type = fog_type(&self.world, self.res.facts(), at, forward);
        let water = self.res.biomes().get(biome).water_fog;
        let (rain, thunder) = if self.dim == Dim::Overworld {
            self.world.weather.levels(self.clock.fraction() as f32)
        } else {
            (0.0, 0.0)
        };
        let title_fog = horizon_client::fog::FogTables::default();
        let tables = if self.title.is_some() {
            &title_fog
        } else {
            self.res.fog()
        };
        horizon_client::fog::setup_fog(
            tables,
            &self.fog,
            &FogInputs {
                fog_type,
                sky,
                water_fog_color: [
                    (water >> 16 & 255) as f32 / 255.0,
                    (water >> 8 & 255) as f32 / 255.0,
                    (water & 255) as f32 / 255.0,
                ],
                biome,
                render_distance_chunks: self.terrain_distance(),
                distant_terrain: self.lod.is_some(),
                cloud_range_chunks: self.settings.cloud_range,
                camera_y: camera.y,
                // level.getMinY(): the Nether and the End start at y 0.
                min_y: if self.dim == Dim::Overworld { MIN_Y } else { 0 },
                flat: false,
                forward: forward.map(|c| c as f32),
                rain,
                thunder,
                skybox: self.dim == Dim::Overworld,
                blindness: None,
                darkness: self.darkness_fog(),
                boss_world_fog: self.dragon_bar_shown(),
            },
        )
    }
    /// What a shader pack's `Frame` uniform reads of the game this frame.
    pub(super) fn shader_inputs(
        &self,
        camera: glam::DVec3,
        partial_tick: f32,
    ) -> horizon_client::gfx::ShaderInputs {
        let (rain, thunder) = if self.dim == Dim::Overworld {
            self.world.weather.levels(self.clock.fraction() as f32)
        } else {
            (0.0, 0.0)
        };
        let at = [camera.x, camera.y, camera.z];
        let day = self.world.day_time;
        let has_sky_light = self.dim == Dim::Overworld;
        // CommonUniforms.getEyeBrightness: the camera entity's eye block.
        let eye = self.eye_position().unwrap_or(at).map(|v| v.floor() as i32);
        let light = self.world.light(eye[0], eye[1], eye[2]);
        let eye_light = [light & 15, if has_sky_light { light >> 4 } else { 0 }];
        horizon_client::gfx::ShaderInputs {
            rain,
            thunder,
            submerged: match fog_type(&self.world, self.res.facts(), at, self.forward()) {
                FogType::Atmospheric => 0,
                FogType::Water => 1,
                FogType::Lava => 2,
                FogType::PowderSnow => 3,
            },
            dimension: match self.dim {
                Dim::Overworld => 0,
                Dim::Nether => -1,
                Dim::End => 1,
            },
            has_sky_light,
            moon_phase: day.div_euclid(24_000).rem_euclid(8) as u8,
            day_fraction: (day.rem_euclid(24_000) as f32 + partial_tick) / 24_000.0,
            partial_tick,
            eye_light,
        }
    }
    /// `SkyRenderer.shouldRenderDarkDisc`: eye below the horizon (y 63) and not under water.
    pub(super) fn dark_disc(&self, camera: glam::DVec3) -> bool {
        camera.y < 63.0
            && fog_type(
                &self.world,
                self.res.facts(),
                [camera.x, camera.y, camera.z],
                self.forward(),
            ) != FogType::Water
    }
    /// `ScreenEffectRenderer` + `Hud` camera overlays for this frame.
    pub(super) fn screen_quads(&self, partial: f32, size: [u32; 2]) -> Vec<ScreenQuad> {
        let Some(p) = self.player.as_ref() else {
            return Vec::new();
        };
        let eye = [
            p.position.x,
            p.position.y + f64::from(p.dimensions.eye_height),
            p.position.z,
        ];
        // LevelExtractor.getViewBlockingState: 8 points around the eye.
        // ScreenEffectRenderer: none of it for a spectator (noPhysics).
        let spectator = self.spectator();
        let mut block_overlay = None;
        let w = f64::from(p.dimensions.width) * 0.8;
        for i in (0..8).filter(|_| !spectator) {
            let t = [
                p.position.x + (f64::from(i & 1) - 0.5) * w,
                eye[1] + (f64::from((i >> 1) & 1) - 0.5) * 0.1,
                p.position.z + (f64::from((i >> 2) & 1) - 0.5) * w,
            ];
            let state = self.world.block(
                t[0].floor() as i32,
                t[1].floor() as i32,
                t[2].floor() as i32,
            );
            let f = self.res.facts().get(state);
            if f.render_shape != horizon_client::blocks::RenderShape::Invisible
                && f.solid_render
                && f.full_collision
            {
                block_overlay = self.particle_sprite(state).map(|(name, _)| name);
                break;
            }
        }
        let in_water =
            fog_type(&self.world, self.res.facts(), eye, self.forward()) == FogType::Water;
        let water =
            (in_water && !spectator).then(|| (self.eye_brightness(eye), self.yaw, self.pitch));
        // Hud.extractCameraOverlays; the spyglass scope (and its easing) is
        // drawn on the HUD by draw_spyglass.
        let fx = &self.screen_fx;
        let overlays = hud_overlays::CameraOverlays {
            vignette: self.settings.vignette.then_some((
                fx.vignette_brightness,
                self.border.warning_strength(p.position.x, p.position.z),
            )),
            first_person: self.camera_type.first_person(),
            scoping: self.scoping(),
            game_time_delta: 0.0,
            equipment: match &self.armor[0] {
                Some(stack) if &*stack.item == "minecraft:carved_pumpkin" => {
                    vec!["misc/pumpkinblur".to_owned()]
                }
                _ => Vec::new(),
            },
            ticks_frozen: p.ticks_frozen,
            percent_frozen: p.percent_frozen(),
            partial,
            portal: [fx.o_portal, fx.portal],
            nausea: 0.0,
            screen_effect_scale: 1.0,
        };
        let mut scope = self.scope_scale;
        let gui_overlays =
            hud_overlays::camera_overlays(size[0] as i32, size[1] as i32, &overlays, &mut scope);
        self.screen_fx.build(&ScreenInputs {
            sprites: &self.gui.atlas.sprites,
            aspect: size[0] as f32 / size[1].max(1) as f32,
            partial,
            block_overlay,
            water,
            on_fire: self.camera_type.first_person()
                && !self.spectator()
                && self.player.as_ref().is_some_and(|p| p.fire_ticks > 0),
            gui_overlays: &gui_overlays,
        })
    }
    /// `Lightmap.getBrightness(getMaxLocalRawBrightness(eye))`.
    fn eye_brightness(&self, eye: [f64; 3]) -> f32 {
        let b = [
            eye[0].floor() as i32,
            eye[1].floor() as i32,
            eye[2].floor() as i32,
        ];
        let packed = self.world.light(b[0], b[1], b[2]);
        let sky = i32::from(packed >> 4) - self.sky_darken();
        let raw = sky.max(i32::from(packed & 15)).clamp(0, 15) as u8;
        lightmap_brightness(if self.dim == Dim::Nether { 0.1 } else { 0.0 }, raw)
    }
    pub(super) fn tick_particles(&mut self) {
        self.fog.tick_flicker();
        let eye = self.eye_position();
        if let Some(e) = eye {
            let brightness = self.eye_brightness(e);
            let inside_portal = self.client_portal_effect();
            self.screen_fx.tick(brightness, inside_portal);
            // LocalPlayer water vision and AtmosphericFogEnvironment rain fog.
            let in_water =
                fog_type(&self.world, self.res.facts(), e, self.forward()) == FogType::Water;
            self.fog.tick(in_water);
            let b = [
                e[0].floor() as i32,
                e[1].floor() as i32,
                e[2].floor() as i32,
            ];
            let rain = if self.dim == Dim::Overworld {
                self.world.weather.levels(1.0).0
            } else {
                0.0
            };
            let biome = self
                .world
                .chunks
                .get(&(b[0] >> 4, b[2] >> 4))
                .map_or(0, |c| c.biome(b[0], b[1], b[2]));
            let rains = self.res.biomes().get(biome).precipitation;
            update_rain_fog(
                &mut self.fog,
                rain,
                self.world.light(b[0], b[1], b[2]) >> 4,
                rains,
                1.0,
            );
        }
        if let Some(e) = eye {
            self.particles.camera = e;
        }
        let (rain, _) = if self.dim == Dim::Overworld {
            self.world.weather.levels(1.0)
        } else {
            (0.0, 0.0)
        };
        let marker = marker_particle_target(self.mode, self.stack_in(self.selected));
        let marker_options = |state| {
            let (sprite, tint) = Self::block_particle_sprite(&self.pack, self.res.facts(), state)?;
            Some(ParticleOptions::block(
                ParticleType::BlockMarker,
                sprite,
                tint,
            ))
        };
        let world = &self.world;
        let facts = self.res.facts();
        let biomes = self.res.biomes();
        let raining = rain > 0.2;
        let raining_at = |p: [i32; 3]| {
            raining
                && motion_blocking(world, facts, p[0], p[2], p[1] + 64) <= p[1]
                && rains_at(world, biomes, p[0], p[1], p[2])
        };
        let ctx = AnimateContext {
            world,
            facts,
            collision: self.collision,
            biomes,
            registry: self.pack.assets.registry,
            raining_at: &raining_at,
            marker: marker.map(|name| (name, &marker_options as &dyn Fn(_) -> _)),
        };
        if let Some(e) = eye {
            let block = [
                e[0].floor() as i32,
                e[1].floor() as i32,
                e[2].floor() as i32,
            ];
            animate_tick(self.res.animate(), &mut self.particles, &ctx, block);
            if rain > 0.0 {
                // ClientLevel.tickWeatherEffects: rain splashes (weather radius 10).
                let mut random = LegacyRandom::new(self.world.game_time.wrapping_mul(312_987_231));
                let radius = 10;
                let diameter = 2 * radius + 1;
                let count = (0.225 * (diameter * diameter) as f32 * rain * rain) as i32;
                for _ in 0..count {
                    let x = block[0] + random.next_int_bound(diameter).unwrap_or(0) - radius;
                    let z = block[2] + random.next_int_bound(diameter).unwrap_or(0) - radius;
                    let top = motion_blocking(world, facts, x, z, block[1] + 64);
                    if top <= MIN_Y
                        || top > block[1] + 10
                        || top < block[1] - 10
                        || !rains_at(world, biomes, x, top, z)
                    {
                        continue;
                    }
                    let below = top - 1;
                    let (bx, bz) = (random.next_double(), random.next_double());
                    let state = world.block(x, below, z);
                    let f = facts.get(state);
                    let block_top = match self.collision.cached_shapes().cached_shape(state) {
                        Ok(Some(shape)) if !shape.is_empty() => {
                            shape.max_at(horizon_core::geometry::Axis::Y, bx, bz)
                        }
                        _ => 0.0,
                    };
                    let fluid_top = if f.fluid == Fluid::None {
                        0.0
                    } else {
                        f64::from(f.fluid_level) / 9.0
                    };
                    let smoke = f.fluid == Fluid::Lava
                        || &*f.name == "minecraft:magma_block"
                        || (f.name.ends_with("campfire")
                            && self.res.animate().is_lit_campfire(state.0));
                    let ty = if smoke {
                        ParticleType::Smoke
                    } else {
                        ParticleType::Rain
                    };
                    let p = [
                        f64::from(x) + bx,
                        f64::from(below) + block_top.max(fluid_top),
                        f64::from(z) + bz,
                    ];
                    self.particles
                        .add(&ParticleOptions::simple(ty), p, [0.0; 3]);
                }
            }
        }
        if let Some(r) = &self.renderer {
            campfire_particles(
                self.res.animate(),
                &mut self.particles,
                &ctx,
                r.block_entities(),
            );
        }
        let level = ParticleLevel {
            world,
            facts,
            collision: self.collision,
            player: self
                .player
                .as_ref()
                .map(|p| ([p.position.x, p.position.y, p.position.z], p.velocity.y)),
        };
        self.particles.tick(&level);
        self.tick_pickups();
        for (name, at, volume, pitch) in std::mem::take(&mut self.particles.sounds) {
            self.audio.play(name, Some(at), volume, pitch);
        }
    }
}
