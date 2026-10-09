//! Mobs in the game loop: ticking with entity snapshots, natural and
//! chunk-generation spawning (`NaturalSpawner`), mob events (attacks,
//! breeding, conversions, drops), projectiles, player interactions
//! (feeding, shearing, milking, spawn eggs, throwing) and rendering.
use super::{Dim, Game, view_vector};
use horizon_client::{
    Result,
    blocks::Fluid,
    collisions::SceneCollisions,
    items::ItemStack,
    mesher::Vertex,
    mobs::{
        Damage, DamageKind, Mob, MobEvent, MobWorld, PlayerView, Seen, SpawnSetup, Species, Who,
        farm::Climate,
        spawning::{self, BiomeFacts},
    },
    projectiles::{Hit, Projectile, ProjectileKind, ProjectileWorld},
};
use horizon_core::{
    geometry::{Aabb, Vec3, traverse_blocks},
    position::BlockPos,
    random::RandomSource,
};
use std::sync::Arc;

/// Mob-system state owned by the game.
#[derive(Default)]
pub(super) struct Creatures {
    pub projectiles: Vec<Projectile>,
    /// Player status effects applied by mobs: (effect, ticks left, amplifier).
    pub player_fire: i32,
    pub pearl_cooldown: u32,
    /// Ticks left of the elder guardian's apparition (MobAppearanceParticle).
    pub elder_apparition: i32,
}
/// MobCategory caps (maxInstancesPerChunk with the 17x17 spawnable chunk area).
const CATEGORIES: [(&str, usize); 7] = [
    ("monster", 70),
    ("creature", 10),
    ("ambient", 15),
    ("axolotls", 5),
    ("underground_water_creature", 5),
    ("water_creature", 5),
    ("water_ambient", 20),
];

struct ProjWorld<'a, 'w> {
    w: &'a MobWorld<'w>,
    targets: Vec<(Who, Aabb)>,
}
impl ProjectileWorld for ProjWorld<'_, '_> {
    fn clip(&self, from: Vec3, to: Vec3) -> Option<(Vec3, [i32; 3], [i32; 3])> {
        let mut scene = SceneCollisions {
            world: self.w.world,
            binding: self.w.collision,
        };
        let r = traverse_blocks(from, to, 256, |p: BlockPos| {
            let state = self.w.world.block(p.x, p.y, p.z);
            if self.w.facts.is_air(state) {
                return None;
            }
            let shape = self
                .w
                .collision
                .collision_shape(&mut scene, state, [p.x, p.y, p.z])
                .ok()?;
            shape.clip(from, to, p)
        });
        let hit = r.ok().flatten()?;
        Some((
            hit.location,
            [hit.block.x, hit.block.y, hit.block.z],
            hit.direction.normal(),
        ))
    }
    fn in_water(&self, at: Vec3) -> bool {
        self.w.fluid(
            at.x.floor() as i32,
            at.y.floor() as i32,
            at.z.floor() as i32,
        ) == Fluid::Water
    }
    fn targets(&self) -> &[(Who, Aabb)] {
        &self.targets
    }
    fn empty(&self, p: [i32; 3]) -> bool {
        self.w.facts.is_air(self.w.world.block(p[0], p[1], p[2]))
    }
    fn solid(&self, p: [i32; 3]) -> bool {
        self.w
            .facts
            .get(self.w.world.block(p[0], p[1], p[2]))
            .full_collision
    }
}
fn seen(m: &Mob) -> Seen {
    Seen {
        id: m.id,
        species: m.species(),
        position: m.position,
        bounds: m.bounds(),
        eye_y: m.eye_y(),
        alive: m.alive(),
        baby: m.is_baby(),
        in_love: m.extra.in_love > 0,
        tame: m.extra.tame,
        hostile: m.hostile(),
        target: m.ai.target,
        health: m.health,
        variant: m.extra.variant,
        color: m.extra.color,
    }
}
impl Game<'_> {
    pub(super) fn player_view(&self) -> Option<PlayerView> {
        let p = self.player.as_ref()?;
        let w = f64::from(p.dimensions.width) / 2.0;
        let bounds = Aabb::new(
            Vec3::new(p.position.x - w, p.position.y, p.position.z - w),
            Vec3::new(
                p.position.x + w,
                p.position.y + f64::from(p.dimensions.height),
                p.position.z + w,
            ),
        );
        Some(PlayerView {
            position: p.position,
            bounds,
            eye: [
                p.position.x,
                p.position.y + f64::from(p.dimensions.eye_height),
                p.position.z,
            ],
            alive: !p.health.dead_or_dying(),
            // EntitySelector.NO_CREATIVE_OR_SPECTATOR.
            creative: self.creative || self.spectator(),
            sneaking: self.keys.contains(&winit::keyboard::KeyCode::ShiftLeft),
            look: view_vector(self.yaw, self.pitch),
            main_hand: self.held().map(|s| s.item.clone()),
            off_hand: None,
            head: self.armor[0].as_ref().map(|s| s.item.clone()),
            armor_cover: self.armor.iter().filter(|a| a.is_some()).count() as f32 / 4.0,
            spectator: self.spectator(),
            gold_armor: self.wearing_gold(),
            hurt_by: self.player_hurt_by(),
            last_hurt: self.combat.last_hurt_mob.map(|(id, at)| (Who::Mob(id), at)),
        })
    }
    /// Biome facts at a position for finalizeSpawn and variant rules.
    pub(super) fn biome_facts(&self, x: i32, y: i32, z: i32) -> BiomeFacts {
        let Some(chunk) = self.world.chunks.get(&(x >> 4, z >> 4)) else {
            return BiomeFacts::default();
        };
        let b = self.res.biomes.get(chunk.biome(x, y, z));
        let tags = &self.res.mob_data;
        let has = |t: &str| tags.biome_has(t, &b.name);
        let climate = if has("minecraft:spawns_warm_variant_farm_animals") {
            Climate::Warm
        } else if has("minecraft:spawns_cold_variant_farm_animals") {
            Climate::Cold
        } else {
            Climate::Temperate
        };
        BiomeFacts {
            climate,
            temperature: b.temperature,
            snowy: has("minecraft:spawns_snow_foxes")
                || b.name.contains("snow")
                || b.name.contains("frozen"),
            swamp: b.name.contains("swamp"),
            ocean: has("minecraft:is_ocean"),
            mushroom: b.name == "minecraft:mushroom_fields",
            desert: b.name == "minecraft:desert",
            jungle: has("minecraft:is_jungle"),
            wolf: horizon_client::mobs::tame::wolf_variant_for(&b.name, &has),
            full_moon: (self.world.day_time / 24000).rem_euclid(8) == 0,
            frog: if has("minecraft:spawns_warm_variant_frogs") {
                1
            } else if has("minecraft:spawns_cold_variant_frogs") {
                2
            } else {
                0
            },
        }
    }
    /// Create a mob of `kind` at a position, finalized like a natural spawn.
    pub(super) fn spawn_mob(&mut self, kind: &str, at: [f64; 3], persistent: bool) -> Option<u32> {
        let kind = if kind.contains(':') {
            kind.to_owned()
        } else {
            format!("minecraft:{kind}")
        };
        let t = self.res.mob_types.get(&kind).cloned()?;
        self.next_entity += 1;
        let yaw = self.world.random.next_float() * 360.0;
        let mut mob = Mob::new(self.next_entity, t, Vec3::new(at[0], at[1], at[2]), yaw);
        mob.persistent = persistent;
        let facts = self.biome_facts(
            at[0].floor() as i32,
            at[1].floor() as i32,
            at[2].floor() as i32,
        );
        spawning::finalize(&mut mob, &facts);
        let id = mob.id;
        self.mobs.push(mob);
        Some(id)
    }
    /// The `summon` command: spawn a mob in front of the player or at a position.
    pub(super) fn summon(&mut self, kind: &str, at: Option<[f64; 3]>, baby: bool) -> Result<u32> {
        let at = match at {
            Some(a) => a,
            None => {
                let p = self.player.as_ref().ok_or("no player yet")?;
                let d = view_vector(self.yaw, 0.0);
                [
                    p.position.x + d[0] * 3.0,
                    p.position.y,
                    p.position.z + d[2] * 3.0,
                ]
            }
        };
        let id = self
            .spawn_mob(kind, at, true)
            .ok_or_else(|| format!("unknown or unsupported mob {kind}"))?;
        if baby && let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
            m.extra.age = -24000;
        }
        Ok(id)
    }
    /// The `mobs` command: nearby mobs with position, health and running goals.
    pub(super) fn control_mobs(&self, radius: f64) -> serde_json::Value {
        let center = self.player.as_ref().map_or(Vec3::ZERO, |p| p.position);
        let list: Vec<serde_json::Value> = self
            .mobs
            .iter()
            .filter(|m| m.distance_sqr(center) <= radius * radius)
            .map(|m| {
                serde_json::json!({
                    "id": m.id,
                    "kind": m.kind.id,
                    "pos": [m.position.x, m.position.y, m.position.z],
                    "health": m.health,
                    "baby": m.is_baby(),
                    "target": m.ai.target.map(|t| format!("{t:?}")),
                    "goals": m.ai.goals.running_names(),
                    "targets": m.ai.targets.running_names(),
                    "path": m.ai.nav.path.as_ref().map(|p| p.nodes.len() - p.next.min(p.nodes.len())),
                    "on_ground": m.on_ground,
                    "variant": m.extra.variant,
                    "color": m.extra.color,
                })
            })
            .collect();
        serde_json::json!({ "count": self.mobs.len(), "near": list, "projectiles": self.creatures.projectiles.len() })
    }
    /// Highest motion-blocking (no leaves) block + 1 (Heightmap MOTION_BLOCKING_NO_LEAVES).
    fn motion_top(&self, x: i32, z: i32) -> i32 {
        let top = if self.dim == Dim::Nether { 127 } else { 319 };
        for y in (horizon_client::chunk::MIN_Y..=top).rev() {
            let f = self.res.facts.get(self.world.block(x, y, z));
            if (f.full_collision || f.fluid != Fluid::None) && !f.name.ends_with("leaves") {
                return y + 1;
            }
        }
        horizon_client::chunk::MIN_Y
    }
    fn weighted_entry(
        &mut self,
        entries: &[horizon_client::biome::SpawnEntry],
    ) -> Option<horizon_client::biome::SpawnEntry> {
        let total: u32 = entries.iter().map(|e| e.weight).sum();
        if total == 0 {
            return None;
        }
        let mut r = self.world.random.next_int_bound(total as i32).ok()? as u32;
        for e in entries {
            if r < e.weight {
                return Some(e.clone());
            }
            r -= e.weight;
        }
        None
    }
    /// NaturalSpawner.spawnMobsForChunkGeneration (creature packs at generation).
    pub(super) fn spawn_generation_creatures(&mut self, cx: i32, cz: i32) {
        let Some(chunk) = self.world.chunks.get(&(cx, cz)).cloned() else {
            return;
        };
        let biome = self.res.biomes.get(chunk.biome(8, 64, 8)).clone();
        let Some(entries) = biome.spawns.get("creature").cloned() else {
            return;
        };
        let (xo, zo) = (cx * 16, cz * 16);
        while self.world.random.next_float() < biome.creature_probability {
            let Some(entry) = self.weighted_entry(&entries) else {
                return;
            };
            let count = entry.min
                + self
                    .world
                    .random
                    .next_int_bound((entry.max - entry.min + 1) as i32)
                    .unwrap_or(0) as u32;
            let mut x = xo + self.world.random.next_int_bound(16).unwrap_or(0);
            let mut z = zo + self.world.random.next_int_bound(16).unwrap_or(0);
            let (sx, sz) = (x, z);
            let Some(kind) = self.res.mob_types.get(&entry.entity).cloned() else {
                continue;
            };
            for _ in 0..count {
                let mut success = false;
                let mut attempts = 0;
                while !success && attempts < 4 {
                    attempts += 1;
                    let mut y = self.motion_top(x, z);
                    let placement = self.res.mob_data.placement(&entry.entity);
                    if placement == horizon_client::mobs::data::Placement::OnGround {
                        let below = self.world.block(x, y - 1, z);
                        if self.res.mob_data.state(below).flags & horizon_client::mobs::data::LAND
                            != 0
                        {
                            y -= 1;
                        }
                    }
                    let ok = {
                        let w = self.mob_world_base(&[]);
                        spawning::placement_ok(kind.species, &w, x, y, z) && {
                            let mut r = self.world.random.clone();

                            spawning::spawn_rules_ok(kind.species, &w, &mut r, x, y, z)
                        }
                    };
                    if ok {
                        let width = kind.width;
                        let fx =
                            f64::from(x).clamp(f64::from(xo) + width, f64::from(xo) + 16.0 - width);
                        let fz =
                            f64::from(z).clamp(f64::from(zo) + width, f64::from(zo) + 16.0 - width);
                        if self
                            .spawn_mob(&entry.entity, [fx, f64::from(y), fz], true)
                            .is_some()
                        {
                            success = true;
                        }
                    }
                    x += self.world.random.next_int_bound(5).unwrap_or(0)
                        - self.world.random.next_int_bound(5).unwrap_or(0);
                    z += self.world.random.next_int_bound(5).unwrap_or(0)
                        - self.world.random.next_int_bound(5).unwrap_or(0);
                    while x < xo || x >= xo + 16 || z < zo || z >= zo + 16 {
                        x = sx + self.world.random.next_int_bound(5).unwrap_or(0)
                            - self.world.random.next_int_bound(5).unwrap_or(0);
                        z = sz + self.world.random.next_int_bound(5).unwrap_or(0)
                            - self.world.random.next_int_bound(5).unwrap_or(0);
                    }
                }
            }
        }
    }
    pub(super) fn mob_world_base<'s>(&'s self, others: &'s [Seen]) -> MobWorld<'s> {
        let (rain, thunder) = self.world.weather.levels(1.0);
        MobWorld {
            world: &self.world,
            facts: &self.res.facts,
            collision: self.collision,
            data: &self.res.mob_data,
            player: self.player_view(),
            others,
            daylight: self.dim != Dim::Nether && self.sky_darken() < 4,
            game_time: self.world.game_time,
            day_time: self.world.day_time,
            raining: rain > 0.2,
            thundering: thunder > 0.9,
            nether: self.dim == Dim::Nether,
            block_tags: &self.res.items.block_tags,
            biomes: &self.res.biomes,
            mob_griefing: self.ui.rules.bool("mob_griefing"),
        }
    }
    /// NaturalSpawner.spawnForChunk over the chunks within 8 of the player.
    fn spawn_natural(&mut self) {
        let Some(p) = self.player.as_ref().map(|p| p.position) else {
            return;
        };
        let (pcx, pcz) = (p.x.floor() as i32 >> 4, p.z.floor() as i32 >> 4);
        let mut counts = [0usize; 7];
        for m in &self.mobs {
            if let Some(i) = CATEGORIES.iter().position(|c| c.0 == m.kind.category) {
                counts[i] += 1;
            }
        }
        let creature_tick = self.world.game_time % 400 == 0;
        let mut chunks: Vec<(i32, i32)> = (-8..=8)
            .flat_map(|dx| (-8..=8).map(move |dz| (pcx + dx, pcz + dz)))
            .filter(|c| self.world.chunks.contains_key(c))
            .collect();
        // ChunkMap: shuffled each tick.
        for i in (1..chunks.len()).rev() {
            let j = self.world.random.next_int_bound(i as i32 + 1).unwrap_or(0) as usize;
            chunks.swap(i, j);
        }
        for (cx, cz) in chunks {
            for (ci, (category, cap)) in CATEGORIES.iter().enumerate() {
                if counts[ci] >= *cap || *category == "creature" && !creature_tick {
                    continue;
                }
                counts[ci] += self.spawn_category_for_chunk(category, cx, cz, p);
            }
        }
    }
    /// NaturalSpawner.spawnCategoryForPosition at a random point in the chunk.
    fn spawn_category_for_chunk(
        &mut self,
        category: &str,
        cx: i32,
        cz: i32,
        player: Vec3,
    ) -> usize {
        let x0 = cx * 16 + self.world.random.next_int_bound(16).unwrap_or(0);
        let z0 = cz * 16 + self.world.random.next_int_bound(16).unwrap_or(0);
        let top = self.motion_top(x0, z0) + 1;
        let min = horizon_client::chunk::MIN_Y;
        let y = min + self.world.random.next_int_bound(top - min + 1).unwrap_or(0);
        if y < min + 1 {
            return 0;
        }
        let start = self.res.facts.get(self.world.block(x0, y, z0));
        if start.full_collision {
            return 0;
        }
        let mut spawned = 0;
        for _ in 0..3 {
            let (mut x, mut z) = (x0, z0);
            let mut entry: Option<horizon_client::biome::SpawnEntry> = None;
            let mut group = (self.world.random.next_float() * 4.0).ceil() as u32;
            let mut cluster = 0;
            let mut i = 0;
            while i < group {
                i += 1;
                x += self.world.random.next_int_bound(6).unwrap_or(0)
                    - self.world.random.next_int_bound(6).unwrap_or(0);
                z += self.world.random.next_int_bound(6).unwrap_or(0)
                    - self.world.random.next_int_bound(6).unwrap_or(0);
                let (fx, fz) = (f64::from(x) + 0.5, f64::from(z) + 0.5);
                let d2 = (fx - player.x).powi(2)
                    + (f64::from(y) - player.y).powi(2)
                    + (fz - player.z).powi(2);
                if d2 <= 576.0 {
                    continue;
                }
                if entry.is_none() {
                    let Some(chunk) = self.world.chunks.get(&(x >> 4, z >> 4)) else {
                        break;
                    };
                    let biome = self.res.biomes.get(chunk.biome(x, y, z)).clone();
                    let list = match self.structure_spawns(category, [x, y, z]) {
                        Some(list) => list,
                        None => match biome.spawns.get(category).cloned() {
                            Some(list) => list,
                            None => break,
                        },
                    };
                    let list: Vec<_> = list
                        .into_iter()
                        .filter(|e| self.res.mob_types.contains_key(&e.entity))
                        .collect();
                    let Some(e) = self.weighted_entry(&list) else {
                        break;
                    };
                    group = e.min
                        + self
                            .world
                            .random
                            .next_int_bound((e.max - e.min + 1) as i32)
                            .unwrap_or(0) as u32;
                    entry = Some(e);
                }
                let e = entry.clone().unwrap_or_else(|| unreachable!());
                let despawn = if category == "water_ambient" {
                    64.0
                } else {
                    128.0
                };
                if d2 > despawn * despawn {
                    continue;
                }
                let Some(kind) = self.res.mob_types.get(&e.entity).cloned() else {
                    break;
                };
                let ok = {
                    let w = self.mob_world_base(&[]);
                    let mut r = self.world.random.clone();
                    let placed = spawning::placement_ok(kind.species, &w, x, y, z)
                        && spawning::spawn_rules_ok(kind.species, &w, &mut r, x, y, z);
                    let half = kind.width / 2.0;
                    let room = w.no_collision(Aabb::new(
                        Vec3::new(fx - half, f64::from(y), fz - half),
                        Vec3::new(fx + half, f64::from(y) + kind.height, fz + half),
                    ));
                    // PathfinderMob.checkSpawnRules: walk target value >= 0 (monsters need darkness).
                    let light_ok = !kind.category.eq("monster")
                        || matches!(
                            kind.species,
                            Species::Slime
                                | Species::Ghast
                                | Species::MagmaCube
                                | Species::Guardian
                                | Species::ElderGuardian
                        )
                        || w.brightness(x, y, z) <= 0.5;
                    let consumed = r;
                    (placed && room && light_ok, consumed)
                };
                self.world.random = ok.1;
                if !ok.0 {
                    continue;
                }
                if d2 > 16384.0 && kind.category != "creature" {
                    continue;
                }
                if self
                    .spawn_mob(&e.entity, [fx, f64::from(y), fz], false)
                    .is_some()
                {
                    cluster += 1;
                    spawned += 1;
                    if spawned >= 4 || cluster >= group.max(1) {
                        return spawned;
                    }
                }
            }
        }
        spawned
    }
    /// Advance every mob one tick, then apply their events.
    pub(super) fn tick_creatures(&mut self) -> Result<()> {
        self.attack_ticks = self.attack_ticks.saturating_add(1);
        let fresh: Vec<(i32, i32)> = self
            .world
            .chunks
            .iter()
            .filter(|(_, c)| c.fresh && c.lit)
            .map(|(k, _)| *k)
            .collect();
        for key in fresh {
            if let Some(c) = self.world.chunks.get_mut(&key) {
                Arc::make_mut(c).fresh = false;
            }
            self.spawn_generation_creatures(key.0, key.1);
        }
        self.spawn_natural();
        let snapshot: Vec<Seen> = self.mobs.iter().map(seen).collect();
        let mut events = Vec::new();
        let mut mobs = std::mem::take(&mut self.mobs);
        let mut dead = Vec::new();
        {
            let w = self.mob_world_base(&snapshot);
            let mut scratch = self.world.random.clone();
            mobs.retain_mut(|m| {
                if !w
                    .world
                    .loaded(m.position.x.floor() as i32, m.position.z.floor() as i32)
                {
                    return !m.hostile() || m.persistent;
                }
                let keep = m.tick(&w, &mut scratch, &mut events);
                if !keep && !m.alive() && !m.removed {
                    dead.push(m.clone());
                }
                keep
            });
            push_apart(&mut mobs);
        }
        self.mobs.append(&mut mobs);
        for m in dead {
            self.mob_died(&m);
        }
        self.handle_mob_events(events)?;
        self.tick_projectiles()?;
        self.tick_fangs();
        self.tick_player_effects();
        self.piglin_pickups();
        Ok(())
    }
    /// LivingEntity.die / dropAllDeathLoot.
    fn mob_died(&mut self, m: &Mob) {
        let by_player = self.last_hurt_by_player.remove(&m.id) || m.ai.last_hurt_by_player > 0;
        if by_player {
            self.stats.add("killed", &m.kind.id, 1);
            self.stats.custom("mob_kills", 1);
            let view = self.mob_entity(m);
            self.trigger_entity("minecraft:player_killed_entity", "entity", &view);
        }
        let held = self.held().cloned();
        let pos = [m.position.x, m.position.y + 0.5, m.position.z];
        if !m.is_baby() {
            let mut drops = self.res.items.entity_drops(
                &m.kind.id,
                by_player,
                held.as_ref(),
                &mut self.world.random,
            );
            if m.species() == Species::Sheep && !m.extra.sheared {
                let color = horizon_client::mobs::render::DYE_NAMES[m.extra.color as usize & 15];
                drops.extend(self.res.items.loot_table(
                    &format!("minecraft:entities/sheep/{color}"),
                    by_player,
                    held.as_ref(),
                    &mut self.world.random,
                ));
            }
            for d in drops {
                if d.count > 0 && &*d.item != "minecraft:air" {
                    self.spawn_item(d, pos, [0.0, 0.2, 0.0], 10);
                }
            }
            // Saddles, horse armour and chests always drop.
            let mut gear: Vec<&str> = Vec::new();
            if m.extra.saddled {
                gear.push("minecraft:saddle");
            }
            if m.extra.chested && horizon_client::mobs::equine::is_equine(m.species()) {
                gear.push("minecraft:chest");
            }
            if horizon_client::mobs::equine::is_equine(m.species())
                && let Some(armor) = m.extra.head.as_deref()
            {
                gear.push(armor);
            }
            for item in gear {
                self.spawn_item(ItemStack::new(item, 1), pos, [0.0, 0.2, 0.0], 10);
            }
            {
                let item = &m.extra.main_hand;
                // Equipment drop chance 8.5% (+ player kill only).
                if let Some(item) = item
                    && by_player
                    && self.world.random.next_float() < 0.085
                {
                    self.spawn_item(
                        ItemStack {
                            item: item.clone(),
                            count: 1,
                            damage: 0,
                            extra: None,
                        },
                        pos,
                        [0.0, 0.2, 0.0],
                        10,
                    );
                }
            }
        }
        if by_player {
            let xp = experience_reward(m, &mut self.world.random);
            if xp > 0 {
                self.spawn_orbs(pos, xp);
            }
        }
        if m.species() == Species::Slime || m.species() == Species::MagmaCube {
            // Slime.remove: split into 2-4 smaller slimes.
            let size = m.extra.size;
            if size > 1 {
                let n = 2 + self.world.random.next_int_bound(3).unwrap_or(0);
                for k in 0..n {
                    let off = |kk: i32| (f64::from(kk % 2) - 0.5) * f64::from(size) / 4.0;
                    let at = [
                        m.position.x + off(k),
                        m.position.y + 0.5,
                        m.position.z + off(k / 2),
                    ];
                    if let Some(id) = self.spawn_mob(&m.kind.id, at, m.persistent)
                        && let Some(c) = self.mobs.iter_mut().find(|c| c.id == id)
                    {
                        horizon_client::mobs::cube::set_size(c, size / 2);
                    }
                }
            }
        }
    }
    pub(super) fn handle_mob_events(&mut self, events: Vec<MobEvent>) -> Result<()> {
        for e in events {
            match e {
                MobEvent::Sound(name, at, volume, pitch) => {
                    self.audio
                        .play(&format!("minecraft:{name}"), Some(at), volume, pitch)
                }
                MobEvent::SoundId(name, at, volume, pitch) => {
                    self.audio.play(&name, Some(at), volume, pitch)
                }
                MobEvent::AttackPlayer(amount, from) => {
                    self.mob_hits_player(amount, from, "minecraft:mob_attack")
                }
                MobEvent::DamagePlayer { amount, from, kind } => {
                    self.mob_hits_player(amount, from, kind)
                }
                MobEvent::ElderCurse { at } => self.elder_curse(at),
                MobEvent::Wololo { mob } => {
                    if let Some(m) = self.mobs.iter_mut().find(|m| m.id == mob) {
                        // DyeColor.RED.
                        m.extra.color = 14;
                    }
                }
                MobEvent::Fangs {
                    at,
                    yaw,
                    delay,
                    owner,
                } => self.add_fangs(at, yaw, delay, owner),
                MobEvent::Shoot(..) => {}
                MobEvent::Explode(at, radius) => {
                    // The exploding mob causes the damage (Creeper.explodeCreeper).
                    if let Some(id) = self
                        .mobs
                        .iter()
                        .filter(|m| {
                            (m.position.x - at[0]).powi(2) + (m.position.z - at[2]).powi(2) < 1.0
                        })
                        .map(|m| m.id)
                        .next()
                    {
                        self.note_attacker("minecraft:explosion", id);
                    }
                    self.explode(at, radius)?
                }
                MobEvent::AttackMob {
                    target,
                    amount,
                    source,
                } => {
                    if let Some(t) = self.mobs.iter_mut().find(|m| m.id == target) {
                        let hit = t.damage(amount, source);
                        if hit {
                            let (kind, center, dead) = (t.kind.clone(), t.center(), !t.alive());
                            let pitch = (self.world.random.next_float()
                                - self.world.random.next_float())
                                * 0.2
                                + 1.0;
                            let s = kind.sounds[if dead { 2 } else { 1 }];
                            if !s.is_empty() {
                                self.audio.play(
                                    &format!("minecraft:{s}"),
                                    Some(center),
                                    1.0,
                                    pitch,
                                );
                            }
                        }
                    }
                }
                MobEvent::FinishAdmiring { mob } => self.finish_admiring(mob),
                MobEvent::Hiss { mob } => {
                    if let Some(m) = self.mobs.iter().find(|m| m.id == mob) {
                        let at = m.center();
                        self.audio
                            .play("minecraft:entity.cat.hiss", Some(at), 1.0, 1.0);
                    }
                }
                MobEvent::Tamed { .. } | MobEvent::Buck { .. } => self.mount_event(&e),
                MobEvent::Launch { target, up } => match target {
                    Who::Player => {
                        if let Some(p) = self.player.as_mut() {
                            p.velocity.y += up;
                        }
                    }
                    Who::Mob(id) => {
                        if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
                            m.velocity.y += up;
                        }
                    }
                },
                MobEvent::HurtSelf { mob, kind, amount } => {
                    if let Some(m) = self.mobs.iter_mut().find(|m| m.id == mob) {
                        m.damage(amount, horizon_client::mobs::Damage::of(kind));
                    }
                }
                MobEvent::SetTarget { mob, target } => {
                    if let Some(m) = self.mobs.iter_mut().find(|m| m.id == mob) {
                        m.ai.target = Some(target);
                    }
                }
                MobEvent::Projectile(p) => self.creatures.projectiles.push(p),
                MobEvent::Spawn(mut m) => {
                    self.next_entity += 1;
                    m.id = self.next_entity;
                    self.mobs.push(*m);
                }
                MobEvent::SpawnKind { kind, at, setup } => {
                    if let Some(id) = self.spawn_mob(&kind, at, setup.persistent) {
                        self.apply_setup(id, &setup);
                    }
                }
                MobEvent::DropItem { item, count, at } => self.spawn_item(
                    ItemStack {
                        item,
                        count,
                        damage: 0,
                        extra: None,
                    },
                    at,
                    [0.0, 0.2, 0.0],
                    10,
                ),
                MobEvent::SetBlock { pos, state } => {
                    self.world.set(pos[0], pos[1], pos[2], state)?
                }
                MobEvent::SetBlockName { pos, name, effect } => {
                    if effect {
                        let old = self.world.block(pos[0], pos[1], pos[2]);
                        self.destroy_particles(pos, old);
                        let s = self.res.facts.get(old).sounds.break_sound.clone();
                        self.audio.play(
                            &s,
                            Some([
                                f64::from(pos[0]) + 0.5,
                                f64::from(pos[1]) + 0.5,
                                f64::from(pos[2]) + 0.5,
                            ]),
                            1.0,
                            0.8,
                        );
                    }
                    if let Some(state) = self.res.assets.registry.default_state(name) {
                        self.world.set(pos[0], pos[1], pos[2], state)?;
                    }
                }
                MobEvent::BreakBlock { pos, drop } => {
                    let old = self.world.block(pos[0], pos[1], pos[2]);
                    self.destroy_particles(pos, old);
                    if drop {
                        let drops = self.res.items.block_drops(
                            self.res.assets.registry,
                            old,
                            None,
                            &mut self.world.random,
                        );
                        for d in drops {
                            self.pop_resource(pos, d);
                        }
                    }
                    self.world.set(pos[0], pos[1], pos[2], self.world.air)?;
                }
                MobEvent::Particles { kind, at, count } => {
                    // Mob particle bursts (hearts, smoke, splashes) around a point.
                    if let Some(ty) = horizon_client::particles::ParticleType::from_name(kind) {
                        let options = horizon_client::particles::ParticleOptions::simple(ty);
                        // Count 0: one particle exactly there, at rest.
                        if count == 0 {
                            self.particles.add(&options, at, [0.0; 3]);
                        }
                        for _ in 0..count {
                            let r = &mut self.world.random;
                            let p = [
                                at[0] + (f64::from(r.next_float()) - 0.5),
                                at[1] + (f64::from(r.next_float()) - 0.5) * 0.5,
                                at[2] + (f64::from(r.next_float()) - 0.5),
                            ];
                            let v = [
                                r.next_gaussian() * 0.02,
                                r.next_gaussian() * 0.02,
                                r.next_gaussian() * 0.02,
                            ];
                            self.particles.add(&options, p, v);
                        }
                    }
                }
                MobEvent::Experience { at, amount } => self.spawn_orbs(at, amount),
                MobEvent::EffectPlayer {
                    effect,
                    ticks,
                    amplifier,
                } => {
                    self.give_player_effect(effect, ticks, amplifier);
                }
                MobEvent::Ignite { target, seconds } => match target {
                    Who::Player => {
                        if !self.creative {
                            self.creatures.player_fire =
                                self.creatures.player_fire.max(seconds * 20);
                        }
                    }
                    Who::Mob(id) => {
                        if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
                            m.fire_ticks = m.fire_ticks.max(seconds * 20);
                        }
                    }
                },
                MobEvent::Breed { parent, partner } => self.breed(parent, partner),
                MobEvent::GiftLoot { table, at } => {
                    let drops =
                        self.res
                            .items
                            .loot_table(table, false, None, &mut self.world.random);
                    for d in drops {
                        self.spawn_item(d, at, [0.0, 0.2, 0.0], 10);
                    }
                }
            }
        }
        Ok(())
    }
    fn apply_setup(&mut self, id: u32, setup: &SpawnSetup) {
        let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) else {
            return;
        };
        if setup.baby {
            m.extra.age = -24000;
        }
        if setup.nausea {
            m.add_effect("minecraft:nausea", 200, 0);
        }
        if setup.size > 0 {
            m.extra.size = setup.size;
        }
        m.yaw = setup.yaw;
        m.body_yaw = setup.yaw;
        m.head_yaw = setup.yaw;
        m.velocity = Vec3::new(setup.velocity[0], setup.velocity[1], setup.velocity[2]);
        if let Some(h) = setup.health {
            m.health = h;
        }
        if let Some(v) = setup.variant {
            m.extra.variant = v;
        }
        if let Some(owner) = setup.owner_mob {
            m.extra.counter2 = owner as i32;
        }
        if let Some(home) = setup.home {
            m.extra.home = Some(home);
        }
        if let Some(life) = setup.life {
            m.extra.egg_time = life;
        }
    }
    /// Animal.spawnChildFromBreeding
    fn breed(&mut self, parent: u32, partner: u32) {
        let Some(p) = self.mobs.iter().find(|m| m.id == parent).cloned() else {
            return;
        };
        // Animal.finalizeSpawnChildFromBreeding: the player who fed them.
        if p.extra.love_player {
            self.stats.custom("animals_bred", 1);
            let parent = self.mob_entity(&p);
            let partner_view = self
                .mobs
                .iter()
                .find(|m| m.id == partner)
                .map(|m| self.mob_entity(m))
                .unwrap_or_else(|| parent.clone());
            let mut child = parent.clone();
            child.baby = true;
            if matches!(
                (
                    p.species(),
                    self.mobs
                        .iter()
                        .find(|m| m.id == partner)
                        .map(|m| m.species())
                ),
                (Species::Horse, Some(Species::Donkey)) | (Species::Donkey, Some(Species::Horse))
            ) {
                child.kind = "minecraft:mule".into();
            }
            self.trigger_bred(&parent, &partner_view, Some(&child));
        }
        if let Some(q) = self.mobs.iter_mut().find(|m| m.id == partner) {
            q.extra.age = 6000;
            q.extra.in_love = 0;
        }
        let q = self.mobs.iter().find(|m| m.id == partner).cloned();
        let coin = self.world.random.next_int_bound(2).unwrap_or(0) == 0;
        let coin2 = self.world.random.next_int_bound(2).unwrap_or(0) == 0;
        // A horse and a donkey have a mule.
        let kind = match (p.species(), q.as_ref().map(|q| q.species())) {
            (Species::Horse, Some(Species::Donkey)) | (Species::Donkey, Some(Species::Horse)) => {
                "minecraft:mule".to_owned()
            }
            _ => p.kind.id.clone(),
        };
        if let Some(id) = self.spawn_mob(&kind, [p.position.x, p.position.y, p.position.z], true)
            && let Some(child) = self.mobs.iter_mut().find(|m| m.id == id)
        {
            child.extra.age = -24000;
            if horizon_client::mobs::equine::is_equine(p.species()) {
                let other = q.clone().unwrap_or_else(|| p.clone());
                let mut r = self.world.random.clone();
                horizon_client::mobs::equine::inherit(child, &p, &other, &mut r);
                // Horse.getBreedOffspring: coat from either parent or new.
                if p.species() == Species::Horse && other.species() == Species::Horse {
                    let c = r.next_int_bound(9).unwrap_or(0);
                    child.extra.variant = if c < 4 {
                        p.extra.variant
                    } else if c < 8 {
                        other.extra.variant
                    } else {
                        r.next_int_bound(7).unwrap_or(0) as u8
                    };
                    let mk = r.next_int_bound(5).unwrap_or(0);
                    child.extra.variant2 = if mk < 2 {
                        p.extra.variant2
                    } else if mk < 4 {
                        other.extra.variant2
                    } else {
                        r.next_int_bound(5).unwrap_or(0) as u8
                    };
                }
                self.world.random = r;
                let xp = 1 + self.world.random.next_int_bound(7).unwrap_or(0) as u32;
                self.spawn_orbs([p.position.x, p.position.y, p.position.z], xp);
                return;
            }
            if p.species() == Species::Cat {
                // Cat.getBreedOffspring.
                let other = q.as_ref().unwrap_or(&p);
                child.extra.variant = if coin {
                    p.extra.variant
                } else {
                    other.extra.variant
                };
                if p.extra.tame {
                    child.extra.tame = true;
                    child.extra.owner = p.extra.owner;
                    child.extra.color = if coin2 {
                        p.extra.color
                    } else {
                        other.extra.color
                    };
                }
                let xp = 1 + self.world.random.next_int_bound(7).unwrap_or(0) as u32;
                self.spawn_orbs([p.position.x, p.position.y, p.position.z], xp);
                return;
            }
            if p.species() == Species::Wolf {
                // Wolf.getBreedOffspring: a parent's variant; tame parents
                // give a tame baby with one of their collar colours.
                let other = q.as_ref().unwrap_or(&p);
                child.extra.variant = if coin {
                    p.extra.variant
                } else {
                    other.extra.variant
                };
                if p.extra.tame {
                    horizon_client::mobs::tame::set_tame(child, true);
                    child.extra.owner = p.extra.owner;
                    child.extra.color = if coin2 {
                        p.extra.color
                    } else {
                        other.extra.color
                    };
                }
                let xp = 1 + self.world.random.next_int_bound(7).unwrap_or(0) as u32;
                self.spawn_orbs([p.position.x, p.position.y, p.position.z], xp);
                return;
            }
            // Offspring inherit a parent's variant (pigs/cows/chickens) or mixed wool.
            child.extra.variant = p.extra.variant;
            if p.species() == Species::Sheep {
                child.extra.color = p.extra.color;
            }
        }
        let xp = 1 + self.world.random.next_int_bound(7).unwrap_or(0) as u32;
        self.spawn_orbs([p.position.x, p.position.y, p.position.z], xp);
    }
    /// `MobEffectUtil.addEffectToPlayersAround` for an elder guardian:
    /// mining fatigue III for five minutes unless a stronger or longer one
    /// is on, with the curse sound and the elder's apparition.
    fn elder_curse(&mut self, at: [f64; 3]) {
        if self.creative || self.spectator() {
            return;
        }
        let Some(p) = self.player.as_ref() else {
            return;
        };
        let d = p.position.subtract(Vec3::new(at[0], at[1], at[2]));
        if d.x * d.x + d.y * d.y + d.z * d.z >= 2500.0 || p.health.dead_or_dying() {
            return;
        }
        let active = self.potions.effects.get("minecraft:mining_fatigue");
        if active.is_some_and(|e| e.amplifier >= 2 && e.duration >= 1200) {
            return;
        }
        let at = [p.position.x, p.position.y, p.position.z];
        self.give_player_effect("minecraft:mining_fatigue", 6000, 2);
        self.audio
            .play("minecraft:entity.elder_guardian.curse", Some(at), 1.0, 1.0);
        self.creatures.elder_apparition = 30;
    }
    /// Damage of a given type to the player from a point.
    pub(super) fn mob_hits_player_kind(&mut self, amount: f32, from: [f64; 3], kind: &str) {
        self.mob_hits_player(amount, from, kind);
    }
    fn mob_hits_player(&mut self, amount: f32, from: [f64; 3], damage_type: &str) {
        if self.creative {
            return;
        }
        let projectile = self
            .res
            .stations
            .enchantments
            .damage_tags
            .get("minecraft:is_projectile")
            .is_some_and(|t| t.contains(damage_type));
        let blocking = self.player_blocking(Some(from), damage_type, false);
        let raised = blocking.is_some();
        // Melee: the attacker stands at `from`.
        let attacker = (!projectile)
            .then(|| {
                self.mobs
                    .iter()
                    .filter(|m| m.alive())
                    .min_by(|a, b| {
                        let d = |m: &horizon_client::mobs::Mob| {
                            (m.position.x - from[0]).powi(2) + (m.position.z - from[2]).powi(2)
                        };
                        d(a).total_cmp(&d(b))
                    })
                    .map(|m| m.id)
            })
            .flatten();
        // The causing entity for the death message.
        let owner = self.combat.projectile_owner.take();
        if let Some(a) = attacker.or(owner) {
            self.note_attacker(damage_type, a);
        }
        let mut outcome = horizon_client::health::BlockOutcome::default();
        if let Some(player) = self.player.as_mut()
            && let Some(id) = self.motion.health.damage.id(damage_type)
            && let Ok(o) = player.hurt_blocking(
                &self.motion.health,
                self.world.health_rules,
                id,
                amount,
                blocking,
            )
            && {
                outcome = o;
                o.hurt
            }
        {
            let (dx, dz) = (player.position.x - from[0], player.position.z - from[2]);
            let l = (dx * dx + dz * dz).sqrt().max(1e-4);
            player.velocity = Vec3::new(
                player.velocity.x / 2.0 + dx / l * 0.4,
                (player.velocity.y / 2.0 + 0.4).min(0.4),
                player.velocity.z / 2.0 + dz / l * 0.4,
            );
            let at = [player.position.x, player.position.y, player.position.z];
            self.audio
                .play("minecraft:entity.player.hurt", Some(at), 1.0, 1.0);
            self.hurt_armor(amount);
            self.thorns(from);
        }
        if raised {
            self.after_block(outcome, amount, attacker, projectile);
        }
    }
    /// A status effect from a mob, arrow or splash potion (`addEffect`).
    pub(super) fn give_player_effect(&mut self, effect: &'static str, ticks: i32, amplifier: i32) {
        if self.creative || self.spectator() {
            return;
        }
        let id = effect.strip_prefix("minecraft:").unwrap_or(effect);
        self.potions
            .effects
            .add(horizon_client::effects::EffectInstance::new(
                id, ticks, amplifier,
            ));
    }
    /// Burning applied to the player by mobs.
    fn tick_player_effects(&mut self) {
        let in_water = self.player.as_ref().is_some_and(|p| {
            let (x, y, z) = (
                p.position.x.floor() as i32,
                p.position.y.floor() as i32,
                p.position.z.floor() as i32,
            );
            self.res.facts.get(self.world.block(x, y, z)).fluid == Fluid::Water
        });
        if self.creatures.player_fire > 0 {
            if in_water {
                self.creatures.player_fire = 0;
            } else {
                if self.creatures.player_fire % 20 == 0 {
                    self.hurt_player_by("minecraft:on_fire", 1.0);
                }
                self.creatures.player_fire -= 1;
            }
        }
        if self.creatures.pearl_cooldown > 0 {
            self.creatures.pearl_cooldown -= 1;
        }
    }
    pub(super) fn hurt_player_by(&mut self, damage_type: &str, amount: f32) {
        if self.creative {
            return;
        }
        if let Some(player) = self.player.as_mut()
            && let Some(id) = self.motion.health.damage.id(damage_type)
        {
            let _ = player.hurt(&self.motion.health, self.world.health_rules, id, amount);
        }
    }
    fn tick_projectiles(&mut self) -> Result<()> {
        let mut projectiles = std::mem::take(&mut self.creatures.projectiles);
        let mut hits = Vec::new();
        {
            let mut targets: Vec<(Who, Aabb)> = self
                .mobs
                .iter()
                .filter(|m| m.alive())
                .map(|m| (Who::Mob(m.id), m.bounds()))
                .collect();
            if let Some(p) = self.player_view()
                && p.alive
            {
                targets.push((Who::Player, p.bounds));
            }
            // Dragon parts and end crystals under reserved ids (game/dragon.rs).
            targets.extend(
                self.special_targets()
                    .into_iter()
                    .map(|(id, b)| (Who::Mob(id), b)),
            );
            let w = self.mob_world_base(&[]);
            let pw = ProjWorld { w: &w, targets };
            projectiles.retain_mut(|p| {
                let (keep, hit) = p.tick(&pw);
                if hit != Hit::None {
                    hits.push((p.clone(), hit));
                }
                keep
            });
        }
        self.creatures.projectiles.append(&mut projectiles);
        for (p, hit) in hits {
            self.projectile_hit(&p, hit)?;
        }
        Ok(())
    }
    fn projectile_hit(&mut self, p: &Projectile, hit: Hit) -> Result<()> {
        self.combat.projectile_owner = match p.owner {
            Some(Who::Mob(id)) => Some(id),
            _ => None,
        };
        let at = match hit {
            Hit::Entity { at, .. } | Hit::Block { at, .. } => at,
            Hit::None => return Ok(()),
        };
        if let Hit::Entity {
            who: Who::Mob(id), ..
        } = hit
            && id >= super::dragon::SPECIAL
        {
            let amount = match p.kind {
                ProjectileKind::Arrow { .. } => p.arrow_damage(&mut self.world.random),
                ProjectileKind::SmallFireball | ProjectileKind::LargeFireball { .. } => 6.0,
                _ => 0.0,
            };
            self.hit_special(id, amount, p.owner == Some(Who::Player));
            return Ok(());
        }
        let atp = [at.x, at.y, at.z];
        let source_kind = match p.kind {
            ProjectileKind::Arrow { .. } => DamageKind::Arrow,
            ProjectileKind::SmallFireball | ProjectileKind::LargeFireball { .. } => {
                DamageKind::Fireball
            }
            _ => DamageKind::Thrown,
        };
        let source = Damage {
            kind: source_kind,
            attacker: p.owner,
            from: Some([p.previous.x, p.previous.y, p.previous.z]),
        };
        match (&p.kind, hit) {
            (ProjectileKind::Arrow { effect, .. }, Hit::Entity { who, .. }) => {
                let amount = p.arrow_damage(&mut self.world.random);
                match who {
                    Who::Player => {
                        if !self.creative {
                            self.mob_hits_player(
                                amount,
                                [p.previous.x, p.previous.y, p.previous.z],
                                "minecraft:arrow",
                            );
                            if let Some((e, t)) = effect {
                                self.give_player_effect(e, *t, 0);
                            }
                            self.audio.play(
                                "minecraft:entity.arrow.hit_player",
                                Some(atp),
                                1.0,
                                1.0,
                            );
                        }
                    }
                    Who::Mob(id) => {
                        if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
                            m.damage(amount, source);
                        }
                        if p.owner == Some(Who::Player) {
                            self.last_hurt_by_player.insert(id);
                        }
                        let pitch = 1.2 / (self.world.random.next_float() * 0.2 + 0.9);
                        self.audio
                            .play("minecraft:entity.arrow.hit", Some(atp), 1.0, pitch);
                    }
                }
            }
            (ProjectileKind::Arrow { .. }, Hit::Block { .. }) => {
                let pitch = 1.2 / (self.world.random.next_float() * 0.2 + 0.9);
                self.audio
                    .play("minecraft:entity.arrow.hit", Some(atp), 1.0, pitch);
            }
            (ProjectileKind::Snowball, hit) => {
                if let Hit::Entity {
                    who: Who::Mob(id), ..
                } = hit
                    && let Some(m) = self.mobs.iter_mut().find(|m| m.id == id)
                {
                    // Snowball.onHitEntity: 3 damage to blazes, 0 otherwise.
                    let amount = if m.species() == Species::Blaze {
                        3.0
                    } else {
                        0.0
                    };
                    m.damage(amount, source);
                }
                self.item_burst(atp, "minecraft:snowball");
            }
            (ProjectileKind::Egg, hit) => {
                if let Hit::Entity {
                    who: Who::Mob(id), ..
                } = hit
                    && let Some(m) = self.mobs.iter_mut().find(|m| m.id == id)
                {
                    m.damage(0.0, source);
                }
                // ThrownEgg.onHit: 1/8 chance of a chick, 1/32 of four.
                if self.world.random.next_int_bound(8).unwrap_or(1) == 0 {
                    let n = if self.world.random.next_int_bound(32).unwrap_or(1) == 0 {
                        4
                    } else {
                        1
                    };
                    for _ in 0..n {
                        if let Some(id) = self.spawn_mob("minecraft:chicken", atp, true)
                            && let Some(c) = self.mobs.iter_mut().find(|m| m.id == id)
                        {
                            c.extra.age = -24000;
                        }
                    }
                }
                self.item_burst(atp, "minecraft:egg");
            }
            (ProjectileKind::EnderPearl, hit) => {
                if let Hit::Entity {
                    who: Who::Mob(id), ..
                } = hit
                    && let Some(m) = self.mobs.iter_mut().find(|m| m.id == id)
                {
                    m.damage(0.0, source);
                }
                if p.owner == Some(Who::Player) && self.player.is_some() {
                    // ThrownEnderpearl: 5% endermite, teleport, 5 damage.
                    if self.world.random.next_float() < 0.05 {
                        self.spawn_mob("minecraft:endermite", atp, false);
                    }
                    if let Some(pl) = self.player.as_mut() {
                        pl.position = at;
                        pl.velocity = Vec3::ZERO;
                        pl.fall_distance = 0.0;
                    }
                    self.previous_position = at;
                    self.hurt_player_by("minecraft:ender_pearl", 5.0);
                    self.audio
                        .play("minecraft:entity.player.teleport", Some(atp), 1.0, 1.0);
                }
            }
            (ProjectileKind::SmallFireball, hit) => match hit {
                Hit::Entity { who, .. } => match who {
                    Who::Player => {
                        if !self.creative {
                            self.creatures.player_fire = self.creatures.player_fire.max(100);
                            self.mob_hits_player(
                                5.0,
                                [p.previous.x, p.previous.y, p.previous.z],
                                "minecraft:fireball",
                            );
                        }
                    }
                    Who::Mob(id) => {
                        if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
                            if !m.kind.flags.fire_immune {
                                m.fire_ticks = m.fire_ticks.max(100);
                            }
                            m.damage(5.0, source);
                        }
                    }
                },
                Hit::Block { pos, face, .. } => {
                    let fire = [pos[0] + face[0], pos[1] + face[1], pos[2] + face[2]];
                    if self
                        .res
                        .facts
                        .is_air(self.world.block(fire[0], fire[1], fire[2]))
                        && let Some(state) =
                            self.res.assets.registry.default_state("minecraft:fire")
                    {
                        self.world.set(fire[0], fire[1], fire[2], state)?;
                    }
                }
                Hit::None => {}
            },
            (ProjectileKind::LargeFireball { power }, hit) => {
                if let Hit::Entity { who, .. } = hit {
                    match who {
                        Who::Player => self.mob_hits_player(
                            6.0,
                            [p.previous.x, p.previous.y, p.previous.z],
                            "minecraft:fireball",
                        ),
                        Who::Mob(id) => {
                            if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
                                m.damage(6.0, source);
                            }
                        }
                    }
                }
                // Fireball explosions are caused by the shooter.
                if let Some(owner) = self.combat.projectile_owner {
                    self.note_attacker("minecraft:explosion", owner);
                }
                self.explode(atp, *power as f32)?;
            }
            (
                ProjectileKind::Potion {
                    effect,
                    ticks,
                    amplifier,
                    ..
                },
                _,
            ) => {
                // AbstractThrownPotion.applySplash: within 4 blocks, scaled by distance.
                if let Some(pv) = self.player_view() {
                    let d2 = (pv.position.x - at.x).powi(2)
                        + (pv.position.y - at.y).powi(2)
                        + (pv.position.z - at.z).powi(2);
                    if d2 < 16.0 && !self.creative {
                        let scale = 1.0 - d2.sqrt() / 4.0;
                        let t = (f64::from(*ticks) * scale + 0.5) as i32;
                        if let Some(owner) = self.combat.projectile_owner {
                            self.note_attacker("minecraft:indirect_magic", owner);
                        }
                        match *effect {
                            "minecraft:instant_damage" => self.hurt_player_by(
                                "minecraft:indirect_magic",
                                6.0 * (1 << *amplifier) as f32 * scale as f32,
                            ),
                            e if t > 20 => self.give_player_effect(e, t, *amplifier),
                            _ => {}
                        }
                    }
                }
                self.audio.play(
                    "minecraft:entity.splash_potion.break",
                    Some(atp),
                    1.0,
                    self.world.random.next_float() * 0.1 + 0.9,
                );
            }
            (ProjectileKind::LlamaSpit, Hit::Entity { who, .. }) => match who {
                Who::Player => self.mob_hits_player(
                    1.0,
                    [p.previous.x, p.previous.y, p.previous.z],
                    "minecraft:spit",
                ),
                Who::Mob(id) => {
                    if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
                        m.damage(1.0, source);
                    }
                }
            },
            // ShulkerBullet.onHitEntity: 4 damage, then levitation for 10 s.
            (ProjectileKind::ShulkerBullet { .. }, Hit::Entity { who, .. }) => match who {
                Who::Player => {
                    if !self.creative {
                        self.mob_hits_player(
                            4.0,
                            [p.previous.x, p.previous.y, p.previous.z],
                            "minecraft:mob_projectile",
                        );
                        self.give_player_effect("minecraft:levitation", 200, 0);
                    }
                }
                Who::Mob(id) => {
                    if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id)
                        && m.damage(4.0, source)
                    {
                        m.extra.levitation = 200;
                    }
                }
            },
            (ProjectileKind::ShulkerBullet { .. }, Hit::Block { .. }) => {
                self.audio
                    .play("minecraft:entity.shulker_bullet.hit", Some(atp), 1.0, 1.0);
                if let Some(ty) = horizon_client::particles::ParticleType::from_name("explosion") {
                    let options = horizon_client::particles::ParticleOptions::simple(ty);
                    for _ in 0..2 {
                        let r = &mut self.world.random;
                        let at = [
                            atp[0] + r.next_gaussian() * 0.2,
                            atp[1] + r.next_gaussian() * 0.2,
                            atp[2] + r.next_gaussian() * 0.2,
                        ];
                        self.particles.add(&options, at, [0.0; 3]);
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
    /// Item break particles (snowballs, eggs).
    fn item_burst(&mut self, at: [f64; 3], item: &str) {
        let sprite = format!("item/{}", item.trim_start_matches("minecraft:"));
        for _ in 0..8 {
            let v = [
                (self.world.random.next_double() - 0.5) * 0.08,
                self.world.random.next_double() * 0.1,
                (self.world.random.next_double() - 0.5) * 0.08,
            ];
            let p = horizon_client::particles::Particle::terrain(
                &mut self.world.random,
                at,
                v,
                sprite.clone(),
                0xFFFFFF,
            );
            self.particles.push(p);
        }
    }
    /// Right click on a mob (Mob.interact / mobInteract). Returns true when consumed.
    pub(super) fn interact_mob(&mut self) -> Result<bool> {
        let Some((index, distance)) = self.mob_hit() else {
            return Ok(false);
        };
        if let Some(hit) = self.pick()
            && let Some(eye) = self.eye_position()
        {
            let d = ((hit.point[0] - eye[0]).powi(2)
                + (hit.point[1] - eye[1]).powi(2)
                + (hit.point[2] - eye[2]).powi(2))
            .sqrt();
            if d < distance {
                return Ok(false);
            }
        }
        if self.name_mob(index) {
            return Ok(true);
        }
        let held = self.held().cloned();
        let item = held
            .as_ref()
            .map(|h| h.item.to_string())
            .unwrap_or_default();
        let creative = self.creative;
        let selected = self.selected;
        // AbstractHorse.mobInteract: feed, equip or ride.
        if self.interact_mount(index, &item) {
            self.swing_arm();
            return Ok(true);
        }
        // PiglinAi.mobInteract: hand over something gold to admire.
        if self.mobs[index].alive() && self.give_piglin(index, &item) {
            self.swing_arm();
            if !creative && let Some(stack) = self.inventory[selected].as_mut() {
                stack.count -= 1;
                if stack.count == 0 {
                    self.inventory[selected] = None;
                }
            }
            return Ok(true);
        }
        let m = &mut self.mobs[index];
        if !m.alive() {
            return Ok(false);
        }
        let species = m.species();
        let center = m.center();
        let mut consume = false;
        let mut give: Option<&str> = None;
        let mut sound: Option<&str> = None;
        let mut handled = true;
        let mut drops: Vec<(String, u32)> = Vec::new();
        let is_food = m.kind.food.contains(&item);
        match species {
            Species::Cow | Species::Mooshroom if item == "minecraft:bucket" && !m.is_baby() => {
                give = Some("minecraft:milk_bucket");
                consume = true;
                sound = Some("entity.cow.milk");
            }
            Species::Mooshroom if item == "minecraft:bowl" && !m.is_baby() => {
                give = Some("minecraft:mushroom_stew");
                consume = true;
                sound = Some("entity.mooshroom.milk");
            }
            Species::Mooshroom if item == "minecraft:shears" && !m.is_baby() => {
                // MushroomCow.shear: becomes a cow and drops five mushrooms.
                let mushroom = if m.extra.variant == 1 {
                    "minecraft:brown_mushroom"
                } else {
                    "minecraft:red_mushroom"
                };
                drops.push((mushroom.into(), 5));
                m.removed = true;
                let setup = SpawnSetup {
                    yaw: m.yaw,
                    persistent: true,
                    ..Default::default()
                };
                let pos = [m.position.x, m.position.y, m.position.z];
                sound = Some("entity.mooshroom.shear");
                self.handle_mob_events(vec![MobEvent::SpawnKind {
                    kind: "minecraft:cow".into(),
                    at: pos,
                    setup,
                }])?;
            }
            Species::Sheep if item == "minecraft:shears" && !m.extra.sheared && !m.is_baby() => {
                m.extra.sheared = true;
                let color = horizon_client::mobs::render::DYE_NAMES[m.extra.color as usize & 15];
                let stacks = self.res.items.loot_table(
                    &format!("minecraft:shearing/sheep/{color}"),
                    false,
                    None,
                    &mut self.world.random,
                );
                drops.extend(stacks.into_iter().map(|s| (s.item.to_string(), s.count)));
                sound = Some("entity.sheep.shear");
                self.damage_held_tool();
            }
            Species::Sheep if item.ends_with("_dye") => {
                let name = item
                    .trim_start_matches("minecraft:")
                    .trim_end_matches("_dye");
                if let Some(c) = horizon_client::mobs::render::DYE_NAMES
                    .iter()
                    .position(|d| *d == name)
                    && m.extra.color as usize != c
                    && !m.extra.sheared
                {
                    m.extra.color = c as u8;
                    consume = true;
                    sound = Some("item.dye.use");
                } else {
                    handled = false;
                }
            }
            // Wolf.mobInteract: heal, dye the collar or sit/stand when tame;
            // a bone tames a wild one a third of the time.
            Species::Wolf if m.extra.tame && is_food && m.health < m.attrs.max_health => {
                let heal = self
                    .res
                    .items
                    .get(&item)
                    .and_then(|i| i.food.as_ref())
                    .map_or(2.0, |f| f.0 as f32);
                m.health = (m.health + heal).min(m.attrs.max_health);
                consume = true;
                sound = Some("entity.wolf.eat");
            }
            Species::Wolf
                if m.extra.tame
                    && item.ends_with("_dye")
                    && horizon_client::mobs::render::DYE_NAMES
                        .iter()
                        .position(|d| {
                            item.trim_start_matches("minecraft:")
                                .trim_end_matches("_dye")
                                == *d
                        })
                        .is_some_and(|c| c as u8 != m.extra.color) =>
            {
                let name = item
                    .trim_start_matches("minecraft:")
                    .trim_end_matches("_dye");
                if let Some(c) = horizon_client::mobs::render::DYE_NAMES
                    .iter()
                    .position(|d| *d == name)
                {
                    m.extra.color = c as u8;
                }
                consume = true;
            }
            Species::Wolf
                if m.extra.tame && !(is_food && m.extra.age == 0 && m.extra.in_love <= 0) =>
            {
                m.extra.sitting = !m.extra.sitting;
                m.ai.jumping = false;
                m.nav_stop();
                m.ai.target = None;
                self.world.modified.insert((
                    m.position.x.floor() as i32 >> 4,
                    m.position.z.floor() as i32 >> 4,
                ));
            }
            Species::Wolf
                if !m.extra.tame
                    && item == "minecraft:bone"
                    && !horizon_client::mobs::tame::angry(m) =>
            {
                consume = true;
                let at = [
                    m.position.x,
                    m.position.y + f64::from(m.height()) * 0.5 + 0.5,
                    m.position.z,
                ];
                let tamed = m.random.next_int_bound(3).unwrap_or(1) == 0;
                if tamed {
                    horizon_client::mobs::tame::set_tame(m, true);
                    m.extra.owner = self.level.player_uuid.or(Some([0, 0, 0, 1]));
                    m.persistent = true;
                    m.nav_stop();
                    m.ai.target = None;
                    m.extra.sitting = true;
                }
                // TamableAnimal.spawnTamingParticles.
                let kind = if tamed { "heart" } else { "smoke" };
                self.handle_mob_events(vec![MobEvent::Particles { kind, at, count: 7 }])?;
                if tamed {
                    let view = self.mob_entity(&self.mobs[index]);
                    self.trigger_entity("minecraft:tame_animal", "entity", &view);
                }
                let m = &self.mobs[index];
                self.world.modified.insert((
                    m.position.x.floor() as i32 >> 4,
                    m.position.z.floor() as i32 >> 4,
                ));
            }
            // Cat.mobInteract: dye, heal or sit when tame; fish may tame a wild one.
            Species::Cat
                if m.extra.tame
                    && item.ends_with("_dye")
                    && horizon_client::mobs::render::DYE_NAMES
                        .iter()
                        .position(|d| {
                            item.trim_start_matches("minecraft:")
                                .trim_end_matches("_dye")
                                == *d
                        })
                        .is_some_and(|c| c as u8 != m.extra.color) =>
            {
                let name = item
                    .trim_start_matches("minecraft:")
                    .trim_end_matches("_dye");
                if let Some(c) = horizon_client::mobs::render::DYE_NAMES
                    .iter()
                    .position(|d| *d == name)
                {
                    m.extra.color = c as u8;
                }
                m.persistent = true;
                consume = true;
            }
            Species::Cat if m.extra.tame && is_food && m.health < m.attrs.max_health => {
                let heal = self
                    .res
                    .items
                    .get(&item)
                    .and_then(|i| i.food.as_ref())
                    .map_or(1.0, |f| f.0 as f32);
                m.health = (m.health + heal).min(m.attrs.max_health);
                consume = true;
                sound = Some("entity.cat.eat");
            }
            Species::Cat
                if m.extra.tame && !(is_food && m.extra.age == 0 && m.extra.in_love <= 0) =>
            {
                m.extra.sitting = !m.extra.sitting;
                self.world.modified.insert((
                    m.position.x.floor() as i32 >> 4,
                    m.position.z.floor() as i32 >> 4,
                ));
            }
            Species::Cat if !m.extra.tame && is_food => {
                consume = true;
                sound = Some("entity.cat.eat");
                m.persistent = true;
                let at = [
                    m.position.x,
                    m.position.y + f64::from(m.height()) * 0.5 + 0.5,
                    m.position.z,
                ];
                let tamed = m.random.next_int_bound(3).unwrap_or(1) == 0;
                if tamed {
                    m.extra.tame = true;
                    m.extra.owner = self.level.player_uuid.or(Some([0, 0, 0, 1]));
                    m.extra.sitting = true;
                }
                let kind = if tamed { "heart" } else { "smoke" };
                self.handle_mob_events(vec![MobEvent::Particles { kind, at, count: 7 }])?;
                if tamed {
                    let view = self.mob_entity(&self.mobs[index]);
                    self.trigger_entity("minecraft:tame_animal", "entity", &view);
                }
                let m = &self.mobs[index];
                self.world.modified.insert((
                    m.position.x.floor() as i32 >> 4,
                    m.position.z.floor() as i32 >> 4,
                ));
            }
            // IronGolem.mobInteract: an iron ingot repairs 25 health.
            Species::IronGolem
                if item == "minecraft:iron_ingot" && m.health < m.attrs.max_health =>
            {
                m.health = (m.health + 25.0).min(m.attrs.max_health);
                consume = true;
                sound = Some("entity.iron_golem.repair");
            }
            Species::Creeper if item == "minecraft:flint_and_steel" => {
                m.extra.flag = true;
                sound = Some("item.flintandsteel.use");
                self.damage_held_tool();
            }
            _ if is_food && m.kind.flags.animal => {
                // Animal.mobInteract: breed adults, speed up babies.
                if m.extra.age == 0 && m.extra.in_love <= 0 {
                    m.extra.in_love = 600;
                    m.extra.love_player = true;
                    consume = true;
                    sound = Some(eat_sound(species));
                } else if m.extra.age < 0 {
                    // AgeableMob.ageUp(getSpeedUpSecondsWhenFeeding(-age)) = 10%.
                    let secs = (-m.extra.age / 20) as f32 * 0.1;
                    m.extra.age = (m.extra.age + (secs as i32) * 20).min(0);
                    consume = true;
                    sound = Some(eat_sound(species));
                } else {
                    handled = false;
                }
            }
            _ => handled = false,
        }
        if !handled {
            return Ok(false);
        }
        self.swing_arm();
        if let Some(s) = sound {
            let pitch =
                (self.world.random.next_float() - self.world.random.next_float()) * 0.2 + 1.0;
            self.audio
                .play(&format!("minecraft:{s}"), Some(center), 1.0, pitch);
        }
        if consume
            && !creative
            && let Some(stack) = self.inventory[selected].as_mut()
        {
            stack.count -= 1;
            if stack.count == 0 {
                self.inventory[selected] = None;
            }
        }
        if let Some(g) = give {
            let stack = ItemStack {
                item: g.into(),
                count: 1,
                damage: 0,
                extra: None,
            };
            if self.inventory[selected].is_none() {
                self.inventory[selected] = Some(stack);
            } else if let Some(rest) = self.give(stack) {
                let at = self.eye_position().unwrap_or(center);
                self.spawn_item(rest, at, [0.0, 0.0, 0.0], 40);
            }
        }
        for (item, count) in drops {
            for _ in 0..count {
                let v = [
                    (self.world.random.next_float() - self.world.random.next_float()) as f64 * 0.1,
                    self.world.random.next_float() as f64 * 0.05,
                    (self.world.random.next_float() - self.world.random.next_float()) as f64 * 0.1,
                ];
                self.spawn_item(
                    ItemStack {
                        item: item.as_str().into(),
                        count: 1,
                        damage: 0,
                        extra: None,
                    },
                    [center[0], center[1] + 0.5, center[2]],
                    v,
                    10,
                );
            }
        }
        Ok(true)
    }
    fn damage_held_tool(&mut self) {
        if self.creative {
            return;
        }
        if let Some(stack) = self.inventory[self.selected].as_mut()
            && let Some(info) = self.res.items.get(&stack.item)
            && info.max_damage > 0
        {
            stack.damage += 1;
            if stack.damage >= info.max_damage {
                self.inventory[self.selected] = None;
            }
        }
    }
    /// Throwables and spawn eggs (SnowballItem/EggItem/EnderpearlItem/SpawnEggItem.use).
    pub(super) fn throw_item(&mut self, stack: &ItemStack) -> Result<bool> {
        let item = &*stack.item;
        let kind = match item {
            "minecraft:snowball" => ProjectileKind::Snowball,
            "minecraft:egg" | "minecraft:blue_egg" | "minecraft:brown_egg" => ProjectileKind::Egg,
            "minecraft:ender_pearl" => {
                if self.creatures.pearl_cooldown > 0 {
                    return Ok(true);
                }
                self.creatures.pearl_cooldown = 20;
                ProjectileKind::EnderPearl
            }
            _ if item.ends_with("_spawn_egg") => return self.use_spawn_egg(stack),
            _ => return Ok(false),
        };
        let Some(eye) = self.eye_position() else {
            return Ok(false);
        };
        let Some(p) = self.player.as_ref() else {
            return Ok(false);
        };
        let (velocity, on_ground) = (p.velocity, p.ground.on_ground);
        let start = Vec3::new(eye[0], eye[1] - 0.1, eye[2]);
        let mut proj = Projectile::new(kind.clone(), start, Some(Who::Player));
        proj.shoot_from_rotation(
            &mut self.world.random,
            self.pitch,
            self.yaw,
            0.0,
            1.5,
            1.0,
            velocity,
            on_ground,
        );
        self.creatures.projectiles.push(proj);
        let sound = match kind {
            ProjectileKind::Snowball => "minecraft:entity.snowball.throw",
            ProjectileKind::Egg => "minecraft:entity.egg.throw",
            _ => "minecraft:entity.ender_pearl.throw",
        };
        let pitch = 0.4 / (self.world.random.next_float() * 0.4 + 0.8);
        self.audio.play(sound, Some(eye), 0.5, pitch);
        self.swing_arm();
        if !self.creative
            && let Some(s) = self.inventory[self.selected].as_mut()
        {
            s.count -= 1;
            if s.count == 0 {
                self.inventory[self.selected] = None;
            }
        }
        Ok(true)
    }
    fn use_spawn_egg(&mut self, stack: &ItemStack) -> Result<bool> {
        let Some(hit) = self.pick() else {
            return Ok(false);
        };
        let kind = stack.item.trim_end_matches("_spawn_egg").to_owned();
        let p = [
            hit.pos[0] + hit.face[0],
            hit.pos[1] + hit.face[1],
            hit.pos[2] + hit.face[2],
        ];
        let at = [
            f64::from(p[0]) + 0.5,
            f64::from(p[1]),
            f64::from(p[2]) + 0.5,
        ];
        if self.spawn_mob(&kind, at, true).is_none() {
            return Ok(false);
        }
        self.swing_arm();
        if !self.creative
            && let Some(s) = self.inventory[self.selected].as_mut()
        {
            s.count -= 1;
            if s.count == 0 {
                self.inventory[self.selected] = None;
            }
        }
        Ok(true)
    }
    /// A beam target's centre (half its height), interpolated.
    fn beam_target(&self, who: Option<Who>, alpha: f64) -> Option<glam::DVec3> {
        match who? {
            Who::Player => {
                let p = self.player.as_ref()?;
                let (a, b) = (self.previous_position, p.position);
                Some(glam::DVec3::new(
                    a.x + (b.x - a.x) * alpha,
                    a.y + (b.y - a.y) * alpha + f64::from(p.dimensions.height) * 0.5,
                    a.z + (b.z - a.z) * alpha,
                ))
            }
            Who::Mob(id) => {
                let m = self.mobs.iter().find(|m| m.id == id)?;
                Some(glam::DVec3::new(
                    m.previous.x + (m.position.x - m.previous.x) * alpha,
                    m.previous.y
                        + (m.position.y - m.previous.y) * alpha
                        + f64::from(m.height()) * 0.5,
                    m.previous.z + (m.position.z - m.previous.z) * alpha,
                ))
            }
        }
    }
    /// Mob models and projectiles for the entity pass.
    pub(super) fn mob_geometry(&self, camera: glam::DVec3, alpha: f64, out: &mut Vec<Vertex>) {
        let atlas = &self.res.atlas;
        let lookup = |name: &str| atlas.sprites.get(name).map(|r| [r.u0, r.v0, r.u1, r.v1]);
        for mob in &self.mobs {
            let d = glam::DVec3::new(mob.position.x, mob.position.y, mob.position.z) - camera;
            if d.length_squared() > 128.0 * 128.0 {
                continue;
            }
            let light = self.world.light(
                mob.position.x.floor() as i32,
                (mob.position.y + mob.eye_height()).floor() as i32,
                mob.position.z.floor() as i32,
            );
            let mut packed = (u32::from(light >> 4) * 16) | ((u32::from(light & 15) * 16) << 8);
            if mob.fire_ticks > 0 || matches!(mob.species(), Species::Blaze | Species::MagmaCube) {
                packed = (15 * 16) | ((15 * 16) << 8);
            }
            horizon_client::mobs::render::render(
                mob,
                alpha as f32,
                camera,
                &lookup,
                packed,
                &self.res.mob_data,
                out,
            );
            // GuardianRenderer: the laser to the locked target.
            if let Some((time, scale)) = horizon_client::mobs::guardian::beam(mob, alpha as f32)
                && let Some(target) = self.beam_target(mob.ai.target, alpha)
                && let Some(uv) = lookup("entity/guardian/guardian_beam")
            {
                let a = alpha;
                let eye = glam::DVec3::new(
                    mob.previous.x + (mob.position.x - mob.previous.x) * a,
                    mob.previous.y + (mob.position.y - mob.previous.y) * a + mob.eye_height(),
                    mob.previous.z + (mob.position.z - mob.previous.z) * a,
                ) - camera;
                horizon_client::mobs::guardian::beam_geometry(
                    eye.as_vec3(),
                    (target - camera).as_vec3(),
                    time,
                    scale,
                    uv,
                    out,
                );
            }
        }
        // Projectiles: arrows as crossed quads, the rest as item billboards.
        let (yaw, pitch) = (self.yaw.to_radians(), self.pitch.to_radians());
        let forward = glam::Vec3::new(
            -yaw.sin() * pitch.cos(),
            -pitch.sin(),
            yaw.cos() * pitch.cos(),
        );
        let right = forward.cross(glam::Vec3::Y).normalize_or_zero();
        let up = right.cross(forward).normalize_or_zero();
        for a in &self.creatures.projectiles {
            let p = glam::DVec3::new(
                a.previous.x + (a.position.x - a.previous.x) * alpha,
                a.previous.y + (a.position.y - a.previous.y) * alpha,
                a.previous.z + (a.position.z - a.previous.z) * alpha,
            ) - camera;
            let c = p.as_vec3();
            let light = self.world.light(
                a.position.x.floor() as i32,
                a.position.y.floor() as i32,
                a.position.z.floor() as i32,
            );
            let mut packed = (u32::from(light >> 4) * 16) | ((u32::from(light & 15) * 16) << 8);
            if a.is_fireball() {
                packed = (15 * 16) | ((15 * 16) << 8);
            }
            if a.is_arrow() {
                let Some(r) = atlas.sprites.get("entity/projectiles/arrow").copied() else {
                    continue;
                };
                let dir = glam::Vec3::new(
                    a.velocity.x as f32,
                    a.velocity.y as f32,
                    a.velocity.z as f32,
                )
                .normalize_or(glam::Vec3::X);
                let dir = if a.stuck {
                    glam::Vec3::new(
                        -a.yaw.to_radians().sin() * a.pitch.to_radians().cos(),
                        a.pitch.to_radians().sin(),
                        a.yaw.to_radians().cos() * a.pitch.to_radians().cos(),
                    )
                    .normalize_or(dir)
                } else {
                    dir
                };
                let side = dir.cross(glam::Vec3::Y).normalize_or(glam::Vec3::Z) * 0.08;
                let upv = dir.cross(side).normalize_or(glam::Vec3::Y) * 0.08;
                let (u1, v1) = (
                    r.u0 + (r.u1 - r.u0) * 0.5,
                    r.v0 + (r.v1 - r.v0) * (5.0 / 32.0),
                );
                for off in [side, upv] {
                    let q = [
                        c - dir * 0.25 - off,
                        c + dir * 0.25 - off,
                        c + dir * 0.25 + off,
                        c - dir * 0.25 + off,
                    ];
                    for (v, uv) in q
                        .iter()
                        .zip([[r.u0, v1], [u1, v1], [u1, r.v0], [r.u0, r.v0]])
                    {
                        out.push(Vertex {
                            pos: v.to_array(),
                            uv,
                            layer: 0,
                            color: 0xFFFF_FFFF,
                            light: packed,
                        });
                    }
                }
                continue;
            }
            let Some(name) = a.item_sprite() else {
                continue;
            };
            let Some(r) = atlas.sprites.get(name).copied() else {
                continue;
            };
            let s = 0.25 * a.sprite_scale();
            for (k, uv) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
                .iter()
                .zip([[r.u0, r.v1], [r.u1, r.v1], [r.u1, r.v0], [r.u0, r.v0]])
            {
                let v = c + right * k.0 * s + up * k.1 * s;
                out.push(Vertex {
                    pos: v.to_array(),
                    uv,
                    layer: 0,
                    color: 0xFFFF_FFFF,
                    light: packed,
                });
            }
        }
    }
}
fn eat_sound(s: Species) -> &'static str {
    match s {
        Species::Pig => "entity.pig.eat",
        Species::Horse | Species::Donkey | Species::Mule => "entity.horse.eat",
        Species::Goat => "entity.goat.eat",
        Species::Fox => "entity.fox.eat",
        Species::Cat => "entity.cat.eat",
        Species::Panda => "entity.panda.eat",
        _ => "entity.generic.eat",
    }
}
/// Mob.getBaseExperienceReward
fn experience_reward(m: &Mob, random: &mut impl RandomSource) -> u32 {
    use Species as S;
    match m.species() {
        S::Blaze | S::Ghast | S::Hoglin | S::Zoglin | S::PiglinBrute => 10,
        S::Slime | S::MagmaCube => m.extra.size.max(1) as u32,
        S::Endermite => 3,
        S::Bat
        | S::Squid
        | S::GlowSquid
        | S::Cod
        | S::Salmon
        | S::TropicalFish
        | S::Pufferfish
        | S::Dolphin => 1 + random.next_int_bound(3).unwrap_or(0) as u32,
        S::IronGolem | S::SnowGolem | S::Villager | S::WanderingTrader => 0,
        _ if m.hostile() => {
            5 + if m.extra.main_hand.is_some() {
                1 + random.next_int_bound(3).unwrap_or(0) as u32
            } else {
                0
            }
        }
        _ if m.is_baby() => 0,
        _ => 1 + random.next_int_bound(3).unwrap_or(0) as u32,
    }
}
/// Entity.push between overlapping mobs (LivingEntity.pushEntities).
fn push_apart(mobs: &mut [Mob]) {
    let n = mobs.len();
    for i in 0..n {
        if !mobs[i].alive() {
            continue;
        }
        let bi = mobs[i].bounds();
        for j in (i + 1)..n {
            if !mobs[j].alive() || !bi.intersects(mobs[j].bounds()) {
                continue;
            }
            let (dx, dz) = (
                mobs[j].position.x - mobs[i].position.x,
                mobs[j].position.z - mobs[i].position.z,
            );
            let mut dd = dx.abs().max(dz.abs());
            if dd >= 0.01 {
                dd = dd.sqrt();
                let (mut xa, mut za) = (dx / dd, dz / dd);
                let inv = (1.0 / dd).min(1.0);
                xa *= inv * 0.05;
                za *= inv * 0.05;
                mobs[i].velocity.x -= xa;
                mobs[i].velocity.z -= za;
                mobs[j].velocity.x += xa;
                mobs[j].velocity.z += za;
            }
        }
    }
}
