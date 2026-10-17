//! Lightning (`ServerLevel.tickThunder`, `LightningBolt`,
//! `LightningBoltRenderer`): strikes on the highest motion-blocking block or
//! a living entity under open sky nearby, skeleton trap horses, fire,
//! `thunderHit` (damage and burning, charged creepers, zombified piglins,
//! witches, mooshroom colours, turtles, burnt items), lightning rods, the sky
//! flash, the sounds and the branching additive bolt.
use super::Game;
use horizon_client::{
    mesher::Vertex,
    mobs::{Damage, DamageKind, MobEvent, SpawnSetup, Species},
};
use horizon_core::{
    position::BlockPos,
    random::{LegacyRandom, RandomSource},
};

pub(super) struct Bolt {
    at: [f64; 3],
    life: i32,
    flashes: i32,
    seed: i64,
    visual_only: bool,
    /// Mooshrooms switch colour once per bolt.
    converted: Vec<u32>,
}
impl Game<'_> {
    /// One `tickThunder` roll per loaded chunk.
    pub(super) fn tick_thunder(&mut self) {
        // isRaining (rain level > 0.2) and isThundering (> 0.9, sky light, no ceiling).
        let (rain, thunder) = self.world.weather.levels(1.0);
        if rain <= 0.2 || thunder <= 0.9 || self.dim != super::Dim::Overworld {
            return;
        }
        let chunks: Vec<(i32, i32)> = self.world.chunks.keys().copied().collect();
        for (cx, cz) in chunks {
            if self.world.random.next_int_bound(100_000).unwrap_or(1) != 0 {
                continue;
            }
            // getBlockRandomPos(minX, 0, minZ, 15)
            let r = self.world.random.next_int();
            let (x, z) = (cx * 16 + (r & 15), cz * 16 + (r >> 16 & 15));
            let Some(pos) = self.lightning_target(x, z) else {
                continue;
            };
            let p = BlockPos::new(pos[0], pos[1], pos[2]);
            if !self.with_level(|b, level, _| b.is_raining_at(level, p)) {
                continue;
            }
            // Skeleton trap: effective difficulty / 100 (regional difficulty approximated).
            let difficulty = f64::from(self.ui.difficulty) * 0.75;
            let below = &self
                .res
                .facts
                .get(self.world.block(pos[0], pos[1] - 1, pos[2]))
                .name;
            let trap = self.ui.rules.bool("spawn_mobs")
                && self.world.random.next_double() < difficulty * 0.01
                && !below.ends_with("lightning_rod");
            if trap {
                let at = [f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2])];
                // ServerLevel.tickChunk: a trap horse waits for a player.
                if let Some(id) = self.spawn_mob("minecraft:skeleton_horse", at, false)
                    && let Some(h) = self.mobs.iter_mut().find(|m| m.id == id)
                {
                    h.extra.trap = true;
                    h.extra.age = 0;
                }
            }
            let at = [
                f64::from(pos[0]) + 0.5,
                f64::from(pos[1]),
                f64::from(pos[2]) + 0.5,
            ];
            self.spawn_bolt(at, trap);
        }
    }
    /// `findLightningTargetAround`: a living entity under open sky in the
    /// column (3 blocks around), else the top of the column.
    fn lightning_target(&mut self, x: i32, z: i32) -> Option<[i32; 3]> {
        if !self.world.loaded(x, z) {
            return None;
        }
        let top =
            self.with_level(|b, level, _| b.motion_blocking_top(level, BlockPos::new(x, 0, z)));
        let sees_sky = |g: &Self, p: [i32; 3]| g.world.light(p[0], p[1], p[2]) >> 4 == 15;
        let mut candidates: Vec<[i32; 3]> = self
            .mobs
            .iter()
            .filter(|m| m.alive())
            .map(|m| {
                [
                    m.position.x.floor() as i32,
                    m.position.y.floor() as i32,
                    m.position.z.floor() as i32,
                ]
            })
            .filter(|p| (p[0] - x).abs() <= 3 && (p[2] - z).abs() <= 3 && p[1] >= top.y - 3)
            .filter(|p| sees_sky(self, *p))
            .collect();
        if let Some(p) = self
            .player
            .as_ref()
            .filter(|p| !p.health.dead_or_dying())
            .map(|p| {
                [
                    p.position.x.floor() as i32,
                    p.position.y.floor() as i32,
                    p.position.z.floor() as i32,
                ]
            })
            && (p[0] - x).abs() <= 3
            && (p[2] - z).abs() <= 3
            && p[1] >= top.y - 3
            && sees_sky(self, p)
        {
            candidates.push(p);
        }
        if !candidates.is_empty() {
            let i = self
                .world
                .random
                .next_int_bound(candidates.len() as i32)
                .unwrap_or(0) as usize;
            return Some(candidates[i]);
        }
        Some([x, top.y, z])
    }
    pub(super) fn spawn_bolt(&mut self, at: [f64; 3], visual_only: bool) {
        if !visual_only {
            let ctx = horizon_client::sculk::Context::default();
            self.post_game_event("minecraft:lightning_strike", at, ctx);
        }
        let r = &mut self.world.random;
        let seed = r.next_long();
        let flashes = r.next_int_bound(3).unwrap_or(0) + 1;
        self.bolts.push(Bolt {
            at,
            life: 2,
            flashes,
            seed,
            visual_only,
            converted: Vec::new(),
        });
    }
    /// `spawnFire`: fire at the strike point and `extra` random neighbours.
    fn bolt_fire(&mut self, at: [f64; 3], extra: i32) {
        if self.ui.difficulty < 2 {
            return;
        }
        let base = [
            at[0].floor() as i32,
            at[1].floor() as i32,
            at[2].floor() as i32,
        ];
        let mut spots = vec![base];
        for _ in 0..extra {
            let r = &mut self.world.random;
            let mut d = || r.next_int_bound(3).unwrap_or(1) - 1;
            spots.push([base[0] + d(), base[1] + d(), base[2] + d()]);
        }
        for p in spots {
            let state = self.world.block(p[0], p[1], p[2]);
            if !self.res.facts.is_air(state) {
                continue;
            }
            // BaseFireBlock.getState: soul fire on soul soil and sand.
            let below = self.res.facts.get(self.world.block(p[0], p[1] - 1, p[2]));
            let name = if matches!(&*below.name, "minecraft:soul_soil" | "minecraft:soul_sand") {
                "minecraft:soul_fire"
            } else {
                "minecraft:fire"
            };
            // FireBlock.canSurvive: a sturdy floor (flammable neighbours aside).
            if below.sturdy_full >> 1 & 1 == 0 {
                continue;
            }
            if let Some(fire) = self.pack.assets.registry.default_state(name) {
                let _ = self.level_set(p, fire);
            }
        }
    }
    /// `LightningBolt.tick` for every bolt.
    pub(super) fn tick_bolts(&mut self) {
        let mut bolts = std::mem::take(&mut self.bolts);
        bolts.retain_mut(|b| {
            if b.life == 2 {
                let pitch = 0.8 + self.world.random.next_float() * 0.2;
                self.audio.play(
                    "minecraft:entity.lightning_bolt.thunder",
                    Some(b.at),
                    10000.0,
                    pitch,
                );
                let pitch = 0.5 + self.world.random.next_float() * 0.2;
                self.audio.play(
                    "minecraft:entity.lightning_bolt.impact",
                    Some(b.at),
                    2.0,
                    pitch,
                );
                if !b.visual_only {
                    self.bolt_fire(b.at, 4);
                    self.strike_rod(b.at);
                }
            }
            b.life -= 1;
            if b.life < 0 {
                if b.flashes == 0 {
                    return false;
                }
                if b.life < -self.world.random.next_int_bound(10).unwrap_or(0) {
                    b.flashes -= 1;
                    b.life = 1;
                    b.seed = self.world.random.next_long();
                    if !b.visual_only {
                        self.bolt_fire(b.at, 0);
                    }
                }
            }
            if b.life >= 0 {
                self.flash = 2;
                if !b.visual_only {
                    self.thunder_hit(b);
                }
            }
            true
        });
        bolts.append(&mut self.bolts);
        self.bolts = bolts;
    }
    /// `LightningRodBlock.onLightningStrike`: powered for 8 ticks.
    fn strike_rod(&mut self, at: [f64; 3]) {
        let p = [
            at[0].floor() as i32,
            (at[1] - 1.0e-6).floor() as i32,
            at[2].floor() as i32,
        ];
        let state = self.world.block(p[0], p[1], p[2]);
        if !self.res.facts.get(state).name.ends_with("lightning_rod") {
            return;
        }
        let registry = self.pack.assets.registry;
        if let Some(mut s) = registry.state(state).cloned() {
            s.properties.insert("powered".into(), "true".into());
            if let Some(powered) = registry.id(&s) {
                // The rod's onPlace schedules the 8-tick unpowering.
                let _ = self.level_set(p, powered);
            }
            // Level event 3002 with the rod's axis: sparks along it.
            let axis = match s.properties.get("facing").map(String::as_str) {
                Some("east" | "west") => 0,
                Some("north" | "south") => 2,
                _ => 1,
            };
            self.level_event_at(3002, p, axis);
        }
    }
    /// `thunderHit` for entities within 3 blocks (up to 9 above).
    fn thunder_hit(&mut self, b: &mut Bolt) {
        let inside = |p: [f64; 3]| {
            (p[0] - b.at[0]).abs() <= 3.0
                && (p[2] - b.at[2]).abs() <= 3.0
                && p[1] >= b.at[1] - 3.0
                && p[1] <= b.at[1] + 9.0
        };
        let damage = Damage {
            kind: DamageKind::Generic,
            attacker: None,
            from: Some(b.at),
        };
        let mut events = Vec::new();
        let peaceful = self.ui.difficulty == 0;
        for m in self.mobs.iter_mut().filter(|m| m.alive()) {
            if !inside([m.position.x, m.position.y, m.position.z]) {
                continue;
            }
            let setup = |m: &horizon_client::mobs::Mob| SpawnSetup {
                yaw: m.yaw,
                persistent: true,
                ..Default::default()
            };
            let at = [m.position.x, m.position.y, m.position.z];
            match m.species() {
                Species::Pig if !peaceful => {
                    m.removed = true;
                    events.push(MobEvent::SpawnKind {
                        kind: "minecraft:zombified_piglin".into(),
                        at,
                        setup: setup(m),
                    });
                    continue;
                }
                Species::Villager if !peaceful => {
                    m.removed = true;
                    events.push(MobEvent::SpawnKind {
                        kind: "minecraft:witch".into(),
                        at,
                        setup: setup(m),
                    });
                    continue;
                }
                Species::Mooshroom => {
                    if !b.converted.contains(&m.id) {
                        b.converted.push(m.id);
                        m.extra.variant = 1 - m.extra.variant.min(1);
                        self.audio
                            .play("minecraft:entity.mooshroom.convert", Some(at), 2.0, 1.0);
                    }
                    continue;
                }
                Species::Turtle => {
                    m.damage(f32::MAX, damage);
                    continue;
                }
                Species::Creeper => m.extra.charged = true,
                _ => {}
            }
            // Entity.thunderHit: burning (8 seconds) and 5 lightning damage.
            m.fire_ticks += 1;
            if m.fire_ticks == 0 {
                m.ignite_for_ticks(160);
            }

            m.damage(5.0, damage);
        }
        if !events.is_empty() {
            let _ = self.handle_mob_events(events);
        }
        if let Some(p) = self.player.as_ref().map(|p| p.position)
            && inside([p.x, p.y, p.z])
        {
            if let Some(p) = self.player.as_mut() {
                p.fire_ticks += 1;
                if p.fire_ticks == 0
                    && let Err(error) = p.ignite_for_ticks(160)
                {
                    self.error = Some(error);
                }
            }
            self.hurt_player_by("minecraft:lightning_bolt", 5.0);
        }
        // Dropped items burn up (ItemEntity health 5), save fire-resistant ones.
        self.dropped.retain(|e| {
            !inside([e.position.x, e.position.y, e.position.z])
                || e.stack.item.contains("netherite")
        });
    }
    /// `LightningBoltRenderer`: 4 layers of a trunk and two branches, 8
    /// segments of 16 blocks, from the bolt's seed.
    pub(super) fn bolt_geometry(&self, camera: glam::DVec3, out: &mut Vec<Vertex>) {
        let white = self.gui.atlas.font.white();
        let uv = [white.u0, white.v0];
        // ARGB 0.45, 0.45, 0.5 at 0.3 alpha, packed RGBA.
        let color = (76u32 << 24) | (127 << 16) | (114 << 8) | 114;
        for b in &self.bolts {
            let o = (glam::DVec3::from_array(b.at) - camera).as_vec3();
            let mut random = LegacyRandom::new(b.seed);
            let (mut xs, mut zs) = ([0f32; 8], [0f32; 8]);
            let (mut x_off, mut z_off) = (0f32, 0f32);
            for i in (0..8).rev() {
                xs[i] = x_off;
                zs[i] = z_off;
                x_off += (random.next_int_bound(11).unwrap_or(5) - 5) as f32;
                z_off += (random.next_int_bound(11).unwrap_or(5) - 5) as f32;
            }
            for layer in 0..4 {
                let mut random = LegacyRandom::new(b.seed);
                for branch in 0..3 {
                    let trunk = branch == 0;
                    let start = 7 - branch;
                    let end = if trunk { 0 } else { start - 3 + 1 };
                    let mut sx = xs[start] - x_off;
                    let mut sz = zs[start] - z_off;
                    for segment in (end..=start).rev() {
                        let (ex, ez) = (sx, sz);
                        let spread = if trunk { 11 } else { 31 };
                        sx += (random.next_int_bound(spread).unwrap_or(0) - spread / 2) as f32;
                        sz += (random.next_int_bound(spread).unwrap_or(0) - spread / 2) as f32;
                        let mut top = 0.1 + layer as f32 * 0.2;
                        let mut bottom = top;
                        if trunk {
                            top *= segment as f32 * 0.1 + 1.0;
                            bottom *= (segment as f32 - 1.0) * 0.1 + 1.0;
                        }
                        for (rx, rz, lx, lz) in [
                            (false, false, true, false),
                            (true, false, true, true),
                            (true, true, false, true),
                            (false, true, false, false),
                        ] {
                            let s = |p: bool, r: f32| if p { r } else { -r };
                            let y0 = segment as f32 * 16.0;
                            let corners = [
                                [sx + s(rx, bottom), y0, sz + s(rz, bottom)],
                                [ex + s(rx, top), y0 + 16.0, ez + s(rz, top)],
                                [ex + s(lx, top), y0 + 16.0, ez + s(lz, top)],
                                [sx + s(lx, bottom), y0, sz + s(lz, bottom)],
                            ];
                            for c in corners {
                                out.push(Vertex {
                                    normal: [0; 4],
                                    pos: (o + glam::Vec3::from_array(c)).to_array(),
                                    uv,
                                    layer: 2,
                                    color,
                                    light: 240 | 240 << 8,
                                });
                            }
                        }
                    }
                }
            }
        }
    }
}
