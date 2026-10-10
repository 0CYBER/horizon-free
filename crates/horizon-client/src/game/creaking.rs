//! The creaking heart block entity (`CreakingHeartBlockEntity`): at night an
//! awake heart in a pale oak trunk spawns a creaking that protects it,
//! hurting the creaking makes the heart emit trails and spread resin over
//! the logs, and breaking the heart tears the creaking down.
use super::Game;
use horizon_client::{
    Result,
    containers::{BlockEntity, CreakingHeart},
    mobs::creaking,
    particles::{ParticleOptions, ParticleType},
};
use horizon_core::{geometry::Aabb, position::BlockPos, random::RandomSource};
use horizon_world::registry::BlockStateId;
use std::collections::{HashSet, VecDeque};

type Pos = (i32, i32, i32);

/// `Creaking.CREAKING_ORANGE` and `CREAKING_GRAY`.
const ORANGE: u32 = 16_545_810;
const GRAY: u32 = 6_250_335;
/// `CreakingHeartBlockEntity.DISTANCE_CREAKING_TOO_FAR`.
const TOO_FAR: f64 = 34.0;

/// `Direction.values()`: down, up, north, south, west, east.
const DIRECTIONS: [[i32; 3]; 6] = [
    [0, -1, 0],
    [0, 1, 0],
    [0, 0, -1],
    [0, 0, 1],
    [-1, 0, 0],
    [1, 0, 0],
];
const FACE_NAMES: [&str; 6] = ["down", "up", "north", "south", "west", "east"];

/// Comparator input of a heart (`getAnalogOutputSignal`).
pub(super) fn heart_signal(world: &horizon_client::world::World, pos: BlockPos) -> Option<i32> {
    match world.block_entities.get(&(pos.x, pos.y, pos.z))? {
        BlockEntity::CreakingHeart(h) => Some(h.output_signal),
        _ => None,
    }
}

impl Game<'_> {
    fn heart_state_name(&self, state: BlockStateId) -> String {
        self.res
            .assets
            .registry
            .state(state)
            .and_then(|s| s.properties.get("creaking_heart_state").cloned())
            .unwrap_or_default()
    }
    /// `Util.shuffledCopy(Direction.values(), random)`.
    fn shuffled_directions(&mut self) -> [usize; 6] {
        let mut order = [0, 1, 2, 3, 4, 5];
        for i in (2..=6).rev() {
            let j = self.world.random.next_int_bound(i as i32).unwrap_or(0) as usize;
            order.swap(i - 1, j);
        }
        order
    }
    fn is_pale_oak_log(&self, p: Pos) -> bool {
        let name = &self.res.facts.get(self.world.block(p.0, p.1, p.2)).name;
        self.res
            .items
            .block_tags
            .get("minecraft:pale_oak_logs")
            .is_some_and(|s| s.contains(&**name))
    }
    fn mob_center(&self, id: u32) -> Option<([f64; 3], Aabb)> {
        let m = self.mobs.iter().find(|m| m.id == id && !m.removed)?;
        Some((m.center(), m.bounds()))
    }
    /// The protector's entity, resolving a UUID read from a save.
    fn heart_protector(&mut self, heart: &mut CreakingHeart) -> Option<u32> {
        let (id, uuid) = heart.creaking?;
        if let Some(id) = id {
            if self.mobs.iter().any(|m| m.id == id && !m.removed) {
                return Some(id);
            }
            // The entity is gone: remember it by UUID until it reappears.
            heart.creaking = Some((None, uuid));
        }
        if let Some(id) = self.mob_id_for_uuid(uuid) {
            heart.creaking = Some((Some(id), uuid));
            return Some(id);
        }
        if heart.ticks_existed >= 30 {
            heart.creaking = None;
        }
        None
    }
    /// `CreakingHeartBlockEntity.removeProtector`: the creaking falls apart.
    fn heart_remove_protector(&mut self, heart: &mut CreakingHeart, by_player: bool) {
        let Some(id) = self.heart_protector(heart) else {
            return;
        };
        if by_player {
            // creakingDeathEffects: a twitching death.
            if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
                creaking::set_tearing_down(m);
                m.health = 0.0;
                m.death_time = m.death_time.max(0);
                let at = m.center();
                self.audio
                    .play("minecraft:entity.creaking.twitch", Some(at), 1.0, 1.0);
            }
        } else if let Some(i) = self.mobs.iter().position(|m| m.id == id) {
            let mut events = Vec::new();
            creaking::tear_down(&mut self.mobs[i], &mut events);
            let _ = self.handle_mob_events(events);
        }
        heart.creaking = None;
    }
    fn heart_distance_to_creaking(&self, id: u32, pos: Pos) -> f64 {
        self.mob_center(id).map_or(0.0, |_| {
            let m = self.mobs.iter().find(|m| m.id == id).unwrap();
            let at = (
                f64::from(pos.0) + 0.5,
                f64::from(pos.1),
                f64::from(pos.2) + 0.5,
            );
            ((m.position.x - at.0).powi(2)
                + (m.position.y - at.1).powi(2)
                + (m.position.z - at.2).powi(2))
            .sqrt()
        })
    }
    /// `emitParticles`: trails between the creaking and the heart.
    fn heart_emit(&mut self, heart: &mut CreakingHeart, pos: Pos, count: i32, towards: bool) {
        let Some(id) = self.heart_protector(heart) else {
            return;
        };
        let Some((_, b)) = self.mob_center(id) else {
            return;
        };
        let (lo, hi) = (b.min_position(), b.max_position());
        for _ in 0..count {
            let r = &mut self.world.random;
            let mut source = [
                lo.x + r.next_double() * (hi.x - lo.x),
                lo.y + r.next_double() * (hi.y - lo.y),
                lo.z + r.next_double() * (hi.z - lo.z),
            ];
            let mut destination = [
                f64::from(pos.0) + r.next_double(),
                f64::from(pos.1) + r.next_double(),
                f64::from(pos.2) + r.next_double(),
            ];
            if towards {
                std::mem::swap(&mut source, &mut destination);
            }
            let duration = r.next_int_bound(40).unwrap_or(0) + 10;
            let color = if towards { ORANGE } else { GRAY };
            self.particles.add(
                &ParticleOptions::trail(destination, color, duration),
                source,
                [0.0; 3],
            );
        }
    }
    /// `CreakingHeartBlockEntity.spreadResin`: a resin clump on the first log
    /// face found within two steps of pale oak logs.
    fn heart_spread_resin(&mut self, pos: Pos) -> Option<Pos> {
        let mut visited: HashSet<Pos> = HashSet::new();
        let mut queue: VecDeque<(Pos, i32)> = VecDeque::new();
        queue.push_back((pos, 0));
        let mut count = 0;
        while let Some((current, depth)) = queue.pop_front() {
            if !visited.insert(current) {
                continue;
            }
            // The node processor.
            if self.is_pale_oak_log(current) {
                for d in self.shuffled_directions() {
                    let n = (
                        current.0 + DIRECTIONS[d][0],
                        current.1 + DIRECTIONS[d][1],
                        current.2 + DIRECTIONS[d][2],
                    );
                    let neighbour = self.world.block(n.0, n.1, n.2);
                    let opposite = d ^ 1;
                    let clump = self.resin_clump_for(neighbour);
                    let Some(clump) = clump else { continue };
                    if self.prop_is(clump, FACE_NAMES[opposite]) {
                        continue;
                    }
                    let Some(placed) = self.with_face(clump, FACE_NAMES[opposite]) else {
                        continue;
                    };
                    let _ = self.level_set([n.0, n.1, n.2], placed);
                    return Some(n);
                }
            }
            count += 1;
            if count >= 64 {
                return None;
            }
            if depth < 2 {
                // The neighbour provider: adjacent pale oak logs.
                for d in self.shuffled_directions() {
                    let n = (
                        current.0 + DIRECTIONS[d][0],
                        current.1 + DIRECTIONS[d][1],
                        current.2 + DIRECTIONS[d][2],
                    );
                    if self.is_pale_oak_log(n) {
                        queue.push_back((n, depth + 1));
                    }
                }
            }
        }
        None
    }
    /// The state a face of resin can be added to: air, a source of water, or
    /// a clump already there.
    fn resin_clump_for(&self, neighbour: BlockStateId) -> Option<BlockStateId> {
        let registry = self.res.assets.registry;
        let clump = registry.default_state("minecraft:resin_clump")?;
        let facts = self.res.facts.get(neighbour);
        if self.res.facts.is_air(neighbour) {
            Some(clump)
        } else if &*facts.name == "minecraft:water"
            && facts.fluid_level == 8
            && !facts.fluid_falling
        {
            let mut s = registry.state(clump)?.clone();
            s.properties.insert("waterlogged".into(), "true".into());
            registry.resolve(&s).ok()
        } else if &*facts.name == "minecraft:resin_clump" {
            Some(neighbour)
        } else {
            None
        }
    }
    fn prop_is(&self, state: BlockStateId, key: &str) -> bool {
        self.res
            .assets
            .registry
            .state(state)
            .and_then(|s| s.properties.get(key))
            .is_some_and(|v| v == "true")
    }
    fn with_face(&self, state: BlockStateId, face: &str) -> Option<BlockStateId> {
        let mut s = self.res.assets.registry.state(state)?.clone();
        s.properties.insert(face.to_owned(), "true".to_owned());
        self.res.assets.registry.resolve(&s).ok()
    }
    /// A creaking bound to the heart at `home` was hurt (`creakingHurt`).
    pub(super) fn creaking_hurt(&mut self, home: [i32; 3], id: u32, by_player: bool) {
        if !by_player {
            return;
        }
        let key = (home[0], home[1], home[2]);
        let Some(BlockEntity::CreakingHeart(mut heart)) = self.world.block_entities.remove(&key)
        else {
            return;
        };
        if self.heart_protector(&mut heart) == Some(id) && heart.emitter <= 0 {
            self.heart_emit(&mut heart, key, 20, false);
            let state = self.world.block(key.0, key.1, key.2);
            if self.heart_state_name(state) == "awake" {
                let clumps = 2 + self.world.random.next_int_bound(2).unwrap_or(0);
                for _ in 0..clumps {
                    if let Some(at) = self.heart_spread_resin(key) {
                        self.audio.play(
                            "minecraft:block.resin.place",
                            Some([
                                f64::from(at.0) + 0.5,
                                f64::from(at.1) + 0.5,
                                f64::from(at.2) + 0.5,
                            ]),
                            1.0,
                            1.0,
                        );
                    }
                }
            }
            heart.emitter = 100;
            heart.emitter_target = self.mob_center(id).map(|(c, _)| c);
        }
        self.world
            .block_entities
            .insert(key, BlockEntity::CreakingHeart(heart));
    }
    /// `Creaking.tearDown`: the crumble particles.
    pub(super) fn creaking_tear_down(&mut self, center: [f64; 3], spread: [f64; 3]) {
        let registry = self.res.assets.registry;
        let crumble = |this: &mut Self, name: &str, props: &[(&str, &str)], count: i32| {
            let Some(mut state) = registry
                .default_state(name)
                .and_then(|s| registry.state(s).cloned())
            else {
                return;
            };
            for (k, v) in props {
                state.properties.insert((*k).to_owned(), (*v).to_owned());
            }
            let Ok(id) = registry.resolve(&state) else {
                return;
            };
            let Some((sprite, tint)) = this.particle_sprite(id) else {
                return;
            };
            for _ in 0..count {
                let r = &mut this.world.random;
                let p = [
                    center[0] + r.next_gaussian() * spread[0],
                    center[1] + r.next_gaussian() * spread[1],
                    center[2] + r.next_gaussian() * spread[2],
                ];
                this.particles.add(
                    &ParticleOptions::block(ParticleType::BlockCrumble, sprite.clone(), tint),
                    p,
                    [0.0; 3],
                );
            }
        };
        crumble(self, "minecraft:pale_oak_wood", &[], 100);
        crumble(
            self,
            "minecraft:creaking_heart",
            &[("creaking_heart_state", "awake")],
            10,
        );
    }
    /// `CreakingHeartBlock.playerWillDestroy`: the heart's creaking twitches to
    /// death and a natural heart drops experience.
    pub(super) fn creaking_heart_broken(&mut self, pos: [i32; 3]) {
        let key = (pos[0], pos[1], pos[2]);
        let Some(BlockEntity::CreakingHeart(mut heart)) = self.world.block_entities.remove(&key)
        else {
            return;
        };
        self.heart_remove_protector(&mut heart, true);
        let state = self.world.block(key.0, key.1, key.2);
        if self.prop_is(state, "natural") && !self.creative && !self.spectator() {
            let xp = 20 + self.world.random.next_int_bound(5).unwrap_or(0) as u32;
            self.spawn_orbs(
                [
                    f64::from(pos[0]) + 0.5,
                    f64::from(pos[1]) + 0.5,
                    f64::from(pos[2]) + 0.5,
                ],
                xp,
            );
        }
        self.world
            .block_entities
            .insert(key, BlockEntity::CreakingHeart(heart));
    }
    /// `SpawnUtil.trySpawnMob(CREAKING, .., 5, 16, 8, ON_TOP_OF_COLLIDER_NO_LEAVES, true)`.
    fn spawn_protector(&mut self, pos: Pos) -> Option<u32> {
        let t = self.res.mob_types.get("minecraft:creaking").cloned()?;
        for _ in 0..5 {
            let r = &mut self.world.random;
            let dx = r.next_int_bound(33).unwrap_or(16) - 16;
            let dz = r.next_int_bound(33).unwrap_or(16) - 16;
            let mut at = (pos.0 + dx, pos.1 + 8, pos.2 + dz);
            if !self.world.loaded(at.0, at.2) {
                continue;
            }
            // moveToPossibleSpawnPosition: scan down for a collider with room above.
            let mut above = self.world.block(at.0, at.1, at.2);
            let mut found = false;
            for _ in 0..=16 {
                at.1 -= 1;
                let current = self.world.block(at.0, at.1, at.2);
                let facts = self.res.facts.get(current);
                let leaves = self
                    .res
                    .items
                    .block_tags
                    .get("minecraft:leaves")
                    .is_some_and(|s| s.contains(&*facts.name));
                let clear_above = self
                    .res
                    .behaviours
                    .collision_shape(above)
                    .map_or(!self.res.facts.get(above).full_collision, |s| s.is_empty());
                if clear_above && !leaves && facts.sturdy_full & (1 << 1) != 0 {
                    at.1 += 1;
                    found = true;
                    break;
                }
                above = current;
            }
            if !found {
                continue;
            }
            // noCollision(spawn box) and no liquid.
            let w = t.width / 2.0;
            let bounds = Aabb::new(
                horizon_core::geometry::Vec3::new(
                    f64::from(at.0) + 0.5 - w,
                    f64::from(at.1),
                    f64::from(at.2) + 0.5 - w,
                ),
                horizon_core::geometry::Vec3::new(
                    f64::from(at.0) + 0.5 + w,
                    f64::from(at.1) + t.height,
                    f64::from(at.2) + 0.5 + w,
                ),
            );
            let mut scene = horizon_client::collisions::SceneCollisions {
                world: &self.world,
                binding: self.collision,
            };
            let free = scene.unobstructed(bounds).unwrap_or(false);
            let wet = self.res.facts.get(self.world.block(at.0, at.1, at.2)).fluid
                != horizon_client::blocks::Fluid::None;
            if !free || wet {
                continue;
            }
            let id = self.spawn_mob(
                "minecraft:creaking",
                [
                    f64::from(at.0) + 0.5,
                    f64::from(at.1),
                    f64::from(at.2) + 0.5,
                ],
                false,
            )?;
            if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
                creaking::set_transient(m, [pos.0, pos.1, pos.2]);
            }
            return Some(id);
        }
        None
    }
    /// `CreakingHeartBlockEntity.computeAnalogOutputSignal`.
    fn heart_output(&mut self, heart: &mut CreakingHeart, pos: Pos) -> i32 {
        if heart.creaking.is_none() {
            return 0;
        }
        let Some(id) = self.heart_protector(heart) else {
            return 0;
        };
        let distance = self.heart_distance_to_creaking(id, pos).clamp(0.0, 32.0) / 32.0;
        15 - (distance * 15.0).floor() as i32
    }
    /// `CreakingHeartBlockEntity.serverTick` for every heart.
    pub(super) fn tick_creaking_hearts(&mut self) -> Result<()> {
        let keys: Vec<Pos> = self
            .world
            .block_entities
            .iter()
            .filter(|(_, e)| matches!(e, BlockEntity::CreakingHeart(_)))
            .map(|(k, _)| *k)
            .collect();
        for key in keys {
            let Some(BlockEntity::CreakingHeart(mut heart)) =
                self.world.block_entities.remove(&key)
            else {
                continue;
            };
            let state = self.world.block(key.0, key.1, key.2);
            if &*self.res.facts.get(state).name != "minecraft:creaking_heart" {
                // The block is gone: preRemoveSideEffects.
                self.heart_remove_protector(&mut heart, false);
                continue;
            }
            if self.heart_state_name(state) != "uprooted" {
                self.heart_tick(&mut heart, key, state)?;
            }
            self.world
                .block_entities
                .insert(key, BlockEntity::CreakingHeart(heart));
        }
        self.scan_for_hearts();
        Ok(())
    }
    fn heart_tick(
        &mut self,
        heart: &mut CreakingHeart,
        key: Pos,
        state: BlockStateId,
    ) -> Result<()> {
        heart.ticks_existed += 1;
        let output = self.heart_output(heart, key);
        if output != heart.output_signal {
            heart.output_signal = output;
            self.container_changed(key);
        }
        if heart.emitter > 0 {
            if heart.emitter > 50 {
                self.heart_emit(heart, key, 1, true);
                self.heart_emit(heart, key, 1, false);
            }
            if heart.emitter % 10 == 0 && heart.emitter_target.is_some() {
                if let Some(id) = self.heart_protector(heart)
                    && let Some((c, _)) = self.mob_center(id)
                {
                    heart.emitter_target = Some(c);
                }
                if let Some(target) = heart.emitter_target {
                    let center = [
                        f64::from(key.0) + 0.5,
                        f64::from(key.1) + 0.5,
                        f64::from(key.2) + 0.5,
                    ];
                    let progress = 0.2 + 0.8 * f64::from(100 - heart.emitter) / 100.0;
                    let at = [
                        (center[0] - target[0]) * progress + target[0],
                        (center[1] - target[1]) * progress + target[1],
                        (center[2] - target[2]) * progress + target[2],
                    ];
                    let volume = heart.emitter as f32 / 2.0 / 100.0 + 0.5;
                    self.audio
                        .play("minecraft:block.creaking_heart.hurt", Some(at), volume, 1.0);
                }
            }
            heart.emitter -= 1;
        }
        let ticker = heart.ticker;
        heart.ticker -= 1;
        if ticker >= 0 {
            return Ok(());
        }
        heart.ticker = self.world.random.next_int_bound(5).unwrap_or(0) + 20;
        // updateCreakingState: logs gone and no creaking means uprooted.
        let active = self.creaking_active();
        let has_logs = self.with_level(|b, level, ticks| {
            b.creaking_heart_has_logs(level, ticks, BlockPos::new(key.0, key.1, key.2), state)
        });
        let wanted = if !has_logs && heart.creaking.is_none() {
            "uprooted"
        } else if active {
            "awake"
        } else {
            "dormant"
        };
        let mut current = self.heart_state_name(state);
        if wanted != current
            && let Some(next) = self.heart_with_state(state, wanted)
        {
            self.level_set([key.0, key.1, key.2], next)?;
            current = wanted.to_owned();
            if wanted == "uprooted" {
                return Ok(());
            }
        }
        if heart.creaking.is_none() {
            if current == "awake" && self.spawning_monsters() {
                let near = self.player.as_ref().is_some_and(|p| {
                    let (dx, dy, dz) = (
                        p.position.x - (f64::from(key.0) + 0.5),
                        p.position.y - (f64::from(key.1) + 0.5),
                        p.position.z - (f64::from(key.2) + 0.5),
                    );
                    dx * dx + dy * dy + dz * dz < 32.0 * 32.0
                }) && !self.spectator();
                if near && let Some(id) = self.spawn_protector(key) {
                    let uuid = self.uuid_for(id);
                    heart.creaking = Some((Some(id), uuid));
                    heart.ticks_existed = 0;
                    let at = self.mob_center(id).map(|(c, _)| c);
                    self.audio
                        .play("minecraft:entity.creaking.spawn", at, 1.0, 1.0);
                    self.audio.play(
                        "minecraft:block.creaking_heart.spawn",
                        Some([
                            f64::from(key.0) + 0.5,
                            f64::from(key.1) + 0.5,
                            f64::from(key.2) + 0.5,
                        ]),
                        1.0,
                        1.0,
                    );
                }
            }
        } else if let Some(id) = self.heart_protector(heart) {
            let persistent = self
                .mobs
                .iter()
                .find(|m| m.id == id)
                .is_some_and(|m| m.persistent);
            let eye = self.player_view().map(|v| v.eye);
            let stuck = self
                .mobs
                .iter_mut()
                .find(|m| m.id == id)
                .is_some_and(|m| creaking::player_is_stuck(m, eye));
            if (!active && !persistent)
                || self.heart_distance_to_creaking(id, key) > TOO_FAR
                || stuck
            {
                self.heart_remove_protector(heart, false);
            }
        }
        Ok(())
    }
    fn heart_with_state(&self, state: BlockStateId, wanted: &str) -> Option<BlockStateId> {
        let mut s = self.res.assets.registry.state(state)?.clone();
        s.properties
            .insert("creaking_heart_state".to_owned(), wanted.to_owned());
        self.res.assets.registry.resolve(&s).ok()
    }
    /// `ServerLevel.isSpawningMonsters`.
    fn spawning_monsters(&self) -> bool {
        self.ui.difficulty != 0 && self.ui.rules.bool("spawn_monsters")
    }
    /// `EnvironmentAttributes.CREAKING_ACTIVE`.
    pub(super) fn creaking_active(&mut self) -> bool {
        self.with_level(|b, level, ticks| b.creaking_active(level, ticks))
    }
    /// Hearts that came with generated or loaded chunks get their block
    /// entity (`newBlockEntity`) the first time their chunk is seen.
    fn scan_for_hearts(&mut self) {
        let Some(heart) = self
            .res
            .assets
            .registry
            .default_state("minecraft:creaking_heart")
        else {
            return;
        };
        let keys: Vec<(i32, i32)> = self
            .world
            .chunks
            .iter()
            .filter(|(k, c)| c.lit && !self.creaking.scanned.contains(k))
            .map(|(k, _)| *k)
            .collect();
        for key in keys {
            self.creaking.scanned.insert(key);
            let Some(chunk) = self.world.chunks.get(&key).cloned() else {
                continue;
            };
            let name = |id: u16| &*self.res.facts.get(BlockStateId(u32::from(id))).name;
            let _ = heart;
            for (si, section) in chunk.sections.iter().enumerate() {
                if section.is_uniform(0) {
                    continue;
                }
                for i in 0..4096 {
                    let id = section.get(i);
                    if name(id) != "minecraft:creaking_heart" {
                        continue;
                    }
                    let (lx, lz, ly) = ((i & 15) as i32, ((i >> 4) & 15) as i32, (i >> 8) as i32);
                    let pos = (
                        key.0 * 16 + lx,
                        horizon_client::chunk::MIN_Y + si as i32 * 16 + ly,
                        key.1 * 16 + lz,
                    );
                    self.world
                        .block_entities
                        .entry(pos)
                        .or_insert_with(|| BlockEntity::CreakingHeart(Box::default()));
                }
            }
        }
        let loaded: HashSet<(i32, i32)> = self.world.chunks.keys().copied().collect();
        self.creaking.scanned.retain(|k| loaded.contains(k));
    }
}

/// What the creaking code remembers between ticks.
#[derive(Default)]
pub(super) struct CreakingState {
    /// Chunks already scanned for heart blocks.
    scanned: HashSet<(i32, i32)>,
}
