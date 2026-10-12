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
    /// The wind charge's use cooldown (ticks).
    pub wind_cooldown: u32,
    /// Ticks left of the elder guardian's apparition (MobAppearanceParticle).
    pub elder_apparition: i32,
    /// The wandering trader spawner's clock.
    pub trader: super::wandering_trader::Spawner,
}
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
fn seen(m: &Mob, player: Option<[i32; 4]>) -> Seen {
    let villager = m.species() == Species::Villager;
    Seen {
        id: m.id,
        species: m.species(),
        position: m.position,
        bounds: m.bounds(),
        eye_y: m.eye_y(),
        alive: m.alive(),
        baby: m.is_baby(),
        // A villager is "in love" when it is willing to breed (`canBreed`).
        in_love: if villager {
            horizon_client::mobs::villager::can_breed(m)
        } else {
            m.extra.in_love > 0
        },
        tame: m.extra.tame,
        hostile: m.hostile(),
        target: m.ai.target,
        health: m.health,
        variant: m.extra.variant,
        color: m.extra.color,
        job: (m.species() == Species::Villager)
            .then_some(m.extra.home)
            .flatten(),
        bed: (m.species() == Species::Villager)
            .then_some(m.extra.bed)
            .flatten(),
        yaw: m.yaw,
        velocity: m.velocity,
        leash: m.extra.leash,
        caravan: horizon_client::mobs::caravan::head_of(m),
        delay: m.extra.ints[5],
        hurt_by: m.ai.last_hurt_by.map(|w| (w, m.ai.last_hurt_by_time)),
        active_raid: horizon_client::mobs::raider::has_active_raid(m),
        player_rep: match (villager, player) {
            (true, Some(uuid)) => {
                horizon_client::mobs::villager::ext(m).map_or(0, |e| e.gossips.reputation(uuid))
            }
            _ => 0,
        },
        pushable: m.pushable(),
        category: m.category(),
        potential_job: horizon_client::mobs::villager_brain::potential_job_site(m),
        meeting: (m.species() == Species::Villager)
            .then_some(m.extra.meeting)
            .flatten(),
        profession: m.extra.variant2,
        xp: horizon_client::mobs::villager::xp(m),
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
            velocity: p.velocity,
            yaw: self.yaw,
            hero: self.potions.effects.get("hero_of_the_village").is_some(),
            uuid: self.level.player_uuid.unwrap_or_default(),
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
            coral_nautilus: has("minecraft:spawns_coral_variant_zombie_nautilus"),
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
        self.spawn_mob_for(kind, at, persistent, false)
    }
    /// `spawn_mob` for a patrol or raid (`EntitySpawnReason.PATROL` / `EVENT`),
    /// which never rolls a patrol leader.
    pub(super) fn spawn_mob_for(
        &mut self,
        kind: &str,
        at: [f64; 3],
        persistent: bool,
        scripted: bool,
    ) -> Option<u32> {
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
        let difficulty = self.local_difficulty(at);
        spawning::finalize(&mut mob, &facts, &difficulty);
        if !scripted {
            let reason = if persistent {
                horizon_client::mobs::raider::Reason::Other
            } else {
                horizon_client::mobs::raider::Reason::Natural
            };
            horizon_client::mobs::raider::finalize(&mut mob, reason);
        }
        let id = mob.id;
        self.mobs.push(mob);
        self.enchant_spawn_equipment(id);
        if !persistent && !scripted {
            self.spawn_jockeys(id, true);
        }
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
            m.extra.age = if m.species() == Species::Sniffer {
                horizon_client::mobs::sniffer::BABY_START_AGE
            } else {
                -24000
            };
        }
        self.spawn_jockeys(id, false);
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
                    "held": m.extra.main_hand.as_deref(),
                    "mount": m.extra.mount,
                    "rider": m.extra.rider,
                    "enchants": horizon_client::mobs::gear::gear(m).map(|g| g.slots.iter().map(|s| s.iter().map(|(e, l)| format!("{e}:{l}")).collect::<Vec<_>>()).collect::<Vec<_>>()),
                    "ints": m.extra.ints,
                    "pois": [m.extra.home, m.extra.bed, m.extra.meeting],
                    "profession": m.extra.variant2,
                    "targets": m.ai.targets.running_names(),
                    "path": m.ai.nav.path.as_ref().map(|p| p.nodes.len() - p.next.min(p.nodes.len())),
                    "on_ground": m.on_ground,
                    "variant": m.extra.variant,
                    "color": m.extra.color,
                    "leash": m.extra.leash.map(|h| format!("{h:?}")),
                    "tame": m.extra.tame,
                    "temper": m.extra.variant3,
                    "chested": m.extra.chested,
                    "head": m.extra.head.as_deref(),
                    "warden": horizon_client::mobs::warden::describe(m),
                    "villager": horizon_client::mobs::villager::ext(m).map(|e| serde_json::json!({
                        "food_level": e.food_level,
                        "inventory": e.inventory.iter().map(|s| format!("{}x{}", s.count, s.item)).collect::<Vec<_>>(),
                        "gossips": e.gossips.len(),
                        "last_slept": e.last_slept,
                        "raid": e.raid,
                    })),
                    "raider": horizon_client::mobs::raider::state(m).map(|s| serde_json::json!({
                        "raid": s.raid_id,
                        "wave": s.wave,
                        "leader": s.patrol_leader,
                        "patrolling": s.patrolling,
                        "patrol_target": s.patrol_target,
                        "vehicle": m.extra.mount,
                    })),
                })
            })
            .collect();
        serde_json::json!({ "count": self.mobs.len(), "near": list, "projectiles": self.creatures.projectiles.len() })
    }
    /// Highest motion-blocking (no leaves) block + 1 (Heightmap MOTION_BLOCKING_NO_LEAVES).
    pub(super) fn motion_top(&self, x: i32, z: i32) -> i32 {
        let top = if self.dim == Dim::Nether { 127 } else { 319 };
        for y in (horizon_client::chunk::MIN_Y..=top).rev() {
            let f = self.res.facts.get(self.world.block(x, y, z));
            if (f.full_collision || f.fluid != Fluid::None) && !f.name.ends_with("leaves") {
                return y + 1;
            }
        }
        horizon_client::chunk::MIN_Y
    }
    pub(super) fn mob_world_base<'s>(&'s self, others: &'s [Seen]) -> MobWorld<'s> {
        let (rain, thunder) = self.world.weather.levels(1.0);
        MobWorld {
            world: &self.world,
            registry: self.res.assets.registry,
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
            end: self.dim == Dim::End,
            world_seed: self.level.seed,
            sea_level: if self.dim == Dim::Nether { 32 } else { 63 },
            difficulty: self.local_difficulty(
                self.player
                    .as_ref()
                    .map_or([0.0; 3], |p| [p.position.x, p.position.y, p.position.z]),
            ),
        }
    }
    /// `ServerLevel.getCurrentDifficultyAt`.
    pub(super) fn local_difficulty(&self, at: [f64; 3]) -> spawning::LocalDifficulty {
        use horizon_world::food::Difficulty;
        let base = match self.world.health_rules.difficulty {
            Difficulty::Peaceful => 0,
            Difficulty::Easy => 1,
            Difficulty::Normal => 2,
            Difficulty::Hard => 3,
        };
        let key = ((at[0].floor() as i32) >> 4, (at[2].floor() as i32) >> 4);
        let (inhabited, moon) = if self.world.chunks.contains_key(&key) {
            let phase = self.world.day_time.div_euclid(24000).rem_euclid(8) as usize;
            (
                self.world.inhabited.get(&key).copied().unwrap_or(0),
                spawning::MOON_BRIGHTNESS[phase],
            )
        } else {
            (0, 0.0)
        };
        spawning::LocalDifficulty::new(base, self.world.day_time, inhabited, moon)
    }
    /// `ServerChunkCache.tickSpawningChunk`: chunks whose centre is within
    /// 128 blocks of the player gain inhabited time.
    fn tick_inhabited(&mut self) {
        let Some(p) = self.player.as_ref().map(|p| p.position) else {
            return;
        };
        let (pcx, pcz) = (p.x.floor() as i32 >> 4, p.z.floor() as i32 >> 4);
        for dx in -8..=8 {
            for dz in -8..=8 {
                let key = (pcx + dx, pcz + dz);
                let (cx, cz) = (f64::from(key.0 * 16 + 8), f64::from(key.1 * 16 + 8));
                if (cx - p.x).powi(2) + (cz - p.z).powi(2) < 128.0 * 128.0
                    && self.world.chunks.contains_key(&key)
                {
                    *self.world.inhabited.entry(key).or_insert(0) += 1;
                }
            }
        }
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
        self.tick_inhabited();
        self.tick_leads();
        self.spawn_natural();
        // ServerChunkCache.tickChunks: the custom spawners tick only with spawn_mobs.
        if self.ui.rules.bool("spawn_mobs") {
            self.tick_wandering_trader();
            self.tick_patrol_spawner();
        }
        self.tick_raids();
        let uuid = self.level.player_uuid;
        let snapshot: Vec<Seen> = self.mobs.iter().map(|m| seen(m, uuid)).collect();
        let mut events = Vec::new();
        let mut mobs = std::mem::take(&mut self.mobs);
        let mut dead = Vec::new();
        {
            let w = self.mob_world_base(&snapshot);
            let mut scratch = self.world.random.clone();
            let mut pushes = Vec::new();
            mobs.retain_mut(|m| {
                if !w
                    .world
                    .loaded(m.position.x.floor() as i32, m.position.z.floor() as i32)
                {
                    return !m.hostile() || m.persistent;
                }
                horizon_client::mobs::apply_mob_pushes(m, &mut pushes);
                let from = events.len();
                let keep = m.tick(&w, &mut scratch, &mut events);
                horizon_client::mobs::take_mob_pushes(&mut events, from, &mut pushes);
                if !keep && !m.alive() && !m.removed {
                    dead.push(m.clone());
                }
                keep
            });
            for m in &mut mobs {
                horizon_client::mobs::apply_mob_pushes(m, &mut pushes);
            }
        }
        self.mobs.append(&mut mobs);
        self.sync_riders();
        self.tick_skeleton_traps();
        self.tick_apparition();
        self.villager_reputation_tick();
        self.villager_pickups();
        for m in dead {
            self.mob_died(&m);
        }
        self.handle_mob_events(events)?;
        self.tick_projectiles()?;
        self.tick_fangs();
        self.tick_allays();
        self.tick_breeze_effects();
        self.tick_sniffer_effects();
        self.tick_player_effects();
        self.piglin_pickups();
        self.sulfur_cube_pickups();
        // Hurt and death sounds of damage dealt outside the mob ticks.
        let mut queued = Vec::new();
        for m in &mut self.mobs {
            queued.append(&mut m.outbox);
        }
        self.handle_mob_events(queued)?;
        Ok(())
    }
    /// LivingEntity.die / dropAllDeathLoot.
    fn mob_died(&mut self, m: &Mob) {
        let by_player = self.last_hurt_by_player.remove(&m.id) || m.ai.last_hurt_by_player > 0;
        let mut dying = m.clone();
        // LivingEntity.die: the death is heard, and a sculk catalyst nearby
        // takes the experience (before the loot is dropped).
        self.sculk_mob_died(m);
        let consumed =
            self.catalyst_takes_experience([m.position.x, m.position.y, m.position.z], |_| {
                if by_player {
                    horizon_client::mobs::drops::base_experience(&mut dying).max(0) as u32
                } else {
                    0
                }
            });
        if by_player {
            self.stats.add("killed", &m.kind.id, 1);
            self.stats.custom("mob_kills", 1);
            self.score_player_killed(super::commands::Target::Mob(m.id));
            let view = self.mob_entity(m);
            self.trigger_entity("minecraft:player_killed_entity", "entity", &view);
        }
        self.score_entity_removed(m.id);
        self.raider_died(m, by_player);
        self.villager_murdered(m, by_player);
        let pos = [m.position.x, m.position.y + 0.5, m.position.z];
        self.drop_death_loot(&mut dying, by_player, consumed, pos);
        if m.species() == Species::CopperGolem {
            // CopperGolem.dropEquipment: the preserved equipment too.
            self.golem_drop_equipment(&mut dying);
        }
        if m.species() == Species::SulfurCube {
            self.split_sulfur_cube(m);
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
    /// `dropAllDeathLoot` through `mobs::drops`: the entity loot table from the
    /// world's random sequence, equipment by drop chance, species extras and
    /// the experience orbs, spawned in order.
    fn drop_death_loot(&mut self, m: &mut Mob, by_player: bool, consumed: bool, pos: [f64; 3]) {
        use horizon_client::mobs::{DamageKind, Who, drops};
        use horizon_world::loot::{LootDamage, LootEntity};
        let Some(loot) = horizon_client::structure_blocks::loot_data(&self.options.resources)
        else {
            return;
        };
        let player = || {
            let mut e = LootEntity::new("minecraft:player");
            if let Some(held) = self.held() {
                e.equipment.push(("mainhand".into(), to_loot(held)));
            }
            e
        };
        let damage = m.last_damage.map(|(d, _)| {
            let source = match d.attacker {
                Some(Who::Player) => Some(player()),
                Some(Who::Mob(id)) => self
                    .mobs
                    .iter()
                    .find(|o| o.id == id)
                    .map(|o| drops::loot_entity(o, None)),
                None => None,
            };
            let direct = match d.kind {
                DamageKind::Arrow => Some(LootEntity {
                    kind: "minecraft:arrow".into(),
                    ..LootEntity::default()
                }),
                DamageKind::Fireball => Some(LootEntity {
                    kind: "minecraft:fireball".into(),
                    ..LootEntity::default()
                }),
                _ => source.clone(),
            };
            let player_source = matches!(d.attacker, Some(Who::Player));
            LootDamage {
                kind: damage_type(d.kind, player_source).into(),
                source,
                direct,
            }
        });
        let vehicle = m
            .extra
            .mount
            .and_then(|id| self.mobs.iter().find(|o| o.id == id))
            .map(|o| LootEntity::new(&o.kind.id));
        let death = drops::Death {
            loot: &loot,
            damage,
            player: by_player.then(player),
            luck: 0.0,
            vehicle,
            mob_drops: self.ui.rules.bool("mob_drops"),
            world_seed: self.level.seed,
        };
        let (items, skulls) = drops::die(m, &death, &mut self.rand_sequences);
        if skulls
            && let Some(Who::Mob(id)) = m.last_damage.and_then(|(d, _)| d.attacker)
            && let Some(creeper) = self.mobs.iter_mut().find(|o| o.id == id)
        {
            // Creeper.droppedSkulls: one head per charged explosion.
            creeper.extra.charged = false;
        }
        let raider = horizon_client::mobs::raider::is_raider(m.species());
        for d in items {
            match d {
                drops::Drop::Item(stack) => {
                    let item = if raider && stack.item == horizon_client::mobs::raider::BANNER {
                        horizon_client::mobs::raider::ominous_banner()
                    } else {
                        from_loot(&stack)
                    };
                    self.spawn_item(item, pos, [0.0, 0.2, 0.0], 10);
                }
                drops::Drop::Orb(value) if !consumed => self.spawn_orbs(pos, value as u32),
                drops::Drop::Orb(_) => {}
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
                MobEvent::LevelEventSound {
                    sound,
                    at,
                    volume,
                    spread,
                } => {
                    let r = &mut self.world.random;
                    let pitch = (r.next_float() - r.next_float()) * spread + 1.0;
                    self.audio
                        .play(&format!("minecraft:{sound}"), Some(at), volume, pitch)
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
                MobEvent::Push { target, velocity } => {
                    let v = Vec3::new(velocity[0], velocity[1], velocity[2]);
                    match target {
                        Who::Player => {
                            if let Some(p) = self.player.as_mut() {
                                p.velocity = p.velocity.add_vector(v);
                            }
                        }
                        Who::Mob(id) => {
                            if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
                                m.velocity = m.velocity.add_vector(v);
                            }
                        }
                    }
                }
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
                    let mut killed = false;
                    if let Some(t) = self.mobs.iter_mut().find(|m| m.id == target) {
                        let hit = t.damage(amount, source);
                        if hit {
                            killed = !t.alive();
                            // The hurt or death sound the mob raised.
                            let center = t.center();
                            for event in std::mem::take(&mut t.outbox) {
                                if let MobEvent::Sound(s, _, volume, pitch) = event {
                                    self.audio.play(
                                        &format!("minecraft:{s}"),
                                        Some(center),
                                        volume,
                                        pitch,
                                    );
                                }
                            }
                        }
                    }
                    if killed && let Some(Who::Mob(killer)) = source.attacker {
                        self.zombie_killed(killer, target);
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
                MobEvent::Gift { target, item } => {
                    if let Some(m) = self.mobs.iter_mut().find(|m| m.id == target)
                        && m.saddle_item().is_none()
                    {
                        m.set_saddle(Some(ItemStack::new(item, 1)));
                    }
                }
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
                MobEvent::Projectile(p) => {
                    let (owner, at) = (p.owner, [p.position.x, p.position.y, p.position.z]);
                    self.creatures.projectiles.push(p);
                    self.projectile_event("minecraft:projectile_shoot", owner, at);
                }
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
                        let drops = self.res.items.block_drops_in(
                            self.res.assets.registry,
                            old,
                            None,
                            &mut self.world.random,
                            Some((&mut self.rand_sequences, self.level.seed)),
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
                MobEvent::LeashBreak { mob, drop, sound } => self.break_leash(mob, drop, sound),
                MobEvent::Vibration { from, to, ticks } => {
                    self.mob_vibration_particle(from, to, ticks)
                }
                MobEvent::DigParticles { at } => self.warden_dig_particles(at),
                MobEvent::Darkness { at, radius } => self.apply_darkness(at, radius),
                MobEvent::CreakingHurt {
                    home,
                    by_player,
                    id,
                } => self.creaking_hurt(home, id, by_player),
                MobEvent::CreakingTearDown { center, spread } => {
                    self.creaking_tear_down(center, spread)
                }
                MobEvent::CureZombieVillager { mob } => self.cure_zombie_villager(mob),
                MobEvent::VillagerMeet { mob, other } => self.villager_meet(mob, other),
                MobEvent::VillagerBirth { parent, partner } => self.villager_birth(parent, partner),
                MobEvent::GolemCheck { mob, needed } => self.villager_golem_check(mob, needed),
                MobEvent::Firework { at, color, flight } => {
                    self.villager_firework(at, color, flight)
                }
                MobEvent::ThrowGift { table, from, to } => self.throw_gift(&table, from, to),
                MobEvent::PatrolTarget { mob, target } => {
                    if let Some(m) = self.mobs.iter_mut().find(|m| m.id == mob) {
                        horizon_client::mobs::raider::set_patrol_target(m, target);
                    }
                }
                MobEvent::RaiderTarget {
                    mob,
                    target,
                    aggressive,
                } => {
                    if let Some(m) = self.mobs.iter_mut().find(|m| m.id == mob) {
                        m.ai.target = Some(target);
                        m.ai.aggressive |= aggressive;
                    }
                }
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
        // Sniffer.spawnChildFromBreeding: an egg instead of a baby.
        if p.species() == Species::Sniffer {
            self.lay_sniffer_egg(&p);
            return;
        }
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
                if horizon_client::mobs::llama::is_llama(p.species()) {
                    horizon_client::mobs::llama::offspring(child, &p, &other, &mut r);
                }
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
            // Nautilus.getBreedOffspring: a tame parent's baby is tame too.
            if p.species() == Species::Nautilus && p.extra.tame {
                child.extra.tame = true;
                child.extra.owner = p.extra.owner;
                child.persistent = true;
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
        self.creatures.elder_apparition = super::mob_blocks::APPARITION_TICKS;
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
        self.creatures.wind_cooldown = self.creatures.wind_cooldown.saturating_sub(1);
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
        let mut deflections: Vec<[f64; 3]> = Vec::new();
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
            let breezes: Vec<u32> = self
                .mobs
                .iter()
                .filter(|m| m.alive() && m.species() == Species::Breeze)
                .map(|m| m.id)
                .collect();
            let jitter = self.world.random.next_float();
            let w = self.mob_world_base(&[]);
            let pw = ProjWorld { w: &w, targets };
            projectiles.retain_mut(|p| {
                let (keep, hit) = p.tick(&pw);
                let wind = matches!(
                    p.kind,
                    ProjectileKind::WindCharge | ProjectileKind::BreezeWindCharge
                );
                // Entity.deflection: a breeze turns every projectile but wind charges away.
                if let Hit::Entity {
                    who: Who::Mob(id),
                    at,
                } = hit
                    && !wind
                    && breezes.contains(&id)
                {
                    super::wind::reverse(p, jitter);
                    deflections.push([at.x, at.y, at.z]);
                    return true;
                }
                // AbstractWindCharge.tick: out of the world's top it bursts.
                if wind && keep && p.position.y > 350.0 {
                    hits.push((
                        p.clone(),
                        Hit::Block {
                            pos: [0; 3],
                            at: p.position,
                            face: [0; 3],
                        },
                    ));
                    return false;
                }
                if hit != Hit::None {
                    hits.push((p.clone(), hit));
                }
                keep
            });
        }
        for at in deflections {
            self.audio
                .play("minecraft:entity.breeze.deflect", Some(at), 1.0, 1.0);
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
        if matches!(
            p.kind,
            ProjectileKind::WindCharge | ProjectileKind::BreezeWindCharge
        ) {
            return self.wind_charge_hit(p, hit);
        }
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
        // Projectile.onHit: the landing is heard.
        self.projectile_event("minecraft:projectile_land", p.owner, atp);
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
                let amount = p.arrow_damage(&mut self.world.random) + p.bonus.damage;
                // Punch pushes along the flight, Flame burns for five seconds.
                let flight = (p.velocity.x * p.velocity.x + p.velocity.z * p.velocity.z).sqrt();
                if p.bonus.punch > 0 && flight > 1e-6 {
                    let k = f64::from(p.bonus.punch) * 0.6 / flight;
                    self.handle_mob_events(vec![MobEvent::Push {
                        target: who,
                        velocity: [p.velocity.x * k, 0.1, p.velocity.z * k],
                    }])?;
                }
                if p.bonus.flame {
                    self.handle_mob_events(vec![MobEvent::Ignite {
                        target: who,
                        seconds: 5,
                    }])?;
                }
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
                // A witch's healing potions reach the raiders around the splash.
                if matches!(
                    *effect,
                    "minecraft:instant_health" | "minecraft:regeneration"
                ) {
                    let center = at;
                    for m in &mut self.mobs {
                        let d2 = m.position.distance_squared(center);
                        if !m.alive()
                            || d2 >= 16.0
                            || !horizon_client::mobs::raider::is_raider(m.species())
                        {
                            continue;
                        }
                        let scale = 1.0 - d2.sqrt() / 4.0;
                        if *effect == "minecraft:instant_health" {
                            let heal = (4 << *amplifier) as f64 * scale;
                            m.health = (m.health + heal as f32).min(m.attrs.max_health);
                        } else {
                            let t = (f64::from(*ticks) * scale + 0.5) as i32;
                            if t > 20 {
                                m.add_effect(effect, t, *amplifier);
                            }
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
            (ProjectileKind::WitherSkull { .. }, hit) => {
                let who = match hit {
                    Hit::Entity { who, .. } => Some(who),
                    _ => None,
                };
                self.wither_skull_hit(p, who, atp)?;
            }
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
        {
            let item = self.held().map(|h| h.item.to_string()).unwrap_or_default();
            if self.interact_leash(index, &item) {
                self.swing_arm();
                return Ok(true);
            }
        }
        let held = self.held().cloned();
        let item = held
            .as_ref()
            .map(|h| h.item.to_string())
            .unwrap_or_default();
        let creative = self.creative;
        let selected = self.selected;
        if self.interact_zombie_villager(index, &item) {
            return Ok(true);
        }
        if self.interact_trader(index, &item)? {
            self.swing_arm();
            return Ok(true);
        }
        if self.interact_allay(index, &item) {
            self.swing_arm();
            return Ok(true);
        }
        if self.interact_sulfur_cube(index, &item) {
            self.swing_arm();
            return Ok(true);
        }
        if self.interact_copper_golem(index, &item) {
            self.swing_arm();
            return Ok(true);
        }
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
            // HappyGhast.canFallInLove: never, so only a ghastling takes food.
            _ if is_food
                && m.kind.flags.animal
                && (species != Species::HappyGhast || m.is_baby()) =>
            {
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
    pub(super) fn damage_held_tool(&mut self) {
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
            "minecraft:wind_charge" => return self.throw_wind_charge(),
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
        self.player_projectile_shot();
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
            self.mob_block_geometry(mob, camera, alpha, packed, out);
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
        self.elder_apparition_geometry(alpha, out);
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
            // WindChargeRenderer.
            if matches!(
                a.kind,
                ProjectileKind::WindCharge | ProjectileKind::BreezeWindCharge
            ) {
                self.wind_charge_geometry(a, c, packed, out);
                continue;
            }
            // WitherSkullRenderer: the skull model facing its flight.
            if let Some(dangerous) = super::wither::skull_kind(a) {
                static SKULL: std::sync::OnceLock<Option<horizon_client::mobs::Part>> =
                    std::sync::OnceLock::new();
                let model = SKULL.get_or_init(|| {
                    horizon_client::mobs::load_model(
                        &self.options.resources,
                        "minecraft:wither_skull#main",
                    )
                });
                let tex = if dangerous {
                    "entity/wither/wither_invulnerable"
                } else {
                    "entity/wither/wither"
                };
                if let (Some(model), Some(r)) = (model, atlas.sprites.get(tex).copied()) {
                    let mut poses = horizon_client::mobs::anim::Poses::new(model);
                    poses.yrot("head", a.yaw.to_radians());
                    poses.xrot("head", a.pitch.to_radians());
                    let base = glam::Mat4::from_translation(c)
                        * glam::Mat4::from_scale(glam::Vec3::new(-1.0, -1.0, 1.0));
                    horizon_client::mobs::render::draw(
                        model,
                        base,
                        &poses,
                        [r.u0, r.v0, r.u1, r.v1],
                        0xFFFF_FFFF,
                        packed,
                        0,
                        out,
                    );
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
        Species::Llama | Species::TraderLlama => "entity.llama.eat",
        Species::Goat => "entity.goat.eat",
        Species::Sniffer => "entity.sniffer.eat",
        Species::Fox => "entity.fox.eat",
        Species::Cat => "entity.cat.eat",
        Species::Panda => "entity.panda.eat",
        _ => "entity.generic.eat",
    }
}
/// Mob.getBaseExperienceReward
/// The damage type id of a Horizon damage kind.
fn damage_type(kind: horizon_client::mobs::DamageKind, player: bool) -> &'static str {
    use horizon_client::mobs::DamageKind as K;
    match kind {
        K::PlayerAttack => "minecraft:player_attack",
        K::MobAttack => "minecraft:mob_attack",
        K::Arrow => "minecraft:arrow",
        K::Thrown => "minecraft:thrown",
        K::Fireball => "minecraft:fireball",
        K::Explosion if player => "minecraft:player_explosion",
        K::Explosion => "minecraft:explosion",
        K::Fall => "minecraft:fall",
        K::OnFire => "minecraft:on_fire",
        K::InFire => "minecraft:in_fire",
        K::Lava => "minecraft:lava",
        K::Drown => "minecraft:drown",
        K::Magic => "minecraft:magic",
        K::Sting => "minecraft:sting",
        K::Cactus => "minecraft:cactus",
        K::Generic => "minecraft:generic",
        K::InWall => "minecraft:in_wall",
        K::Thorns => "minecraft:thorns",
        K::SonicBoom => "minecraft:sonic_boom",
        K::OutOfWorld => "minecraft:out_of_world",
    }
}
/// A Horizon item stack as a loot stack (id, count, damage, enchantments).
fn to_loot(stack: &ItemStack) -> horizon_world::loot::LootStack {
    horizon_world::loot::LootStack {
        item: stack.item.to_string(),
        count: stack.count as i32,
        damage: stack.damage as i32,
        enchantments: stack
            .enchantments()
            .iter()
            .map(|(e, l)| (e.to_string(), *l as i32))
            .collect(),
        potion: None,
        map: false,
        ominous_amplifier: None,
    }
}
/// A generated loot stack as a Horizon item stack.
fn from_loot(stack: &horizon_world::loot::LootStack) -> ItemStack {
    let mut item = ItemStack {
        item: Arc::from(stack.item.as_str()),
        count: stack.count.max(1) as u32,
        damage: stack.damage.max(0) as u32,
        extra: None,
    };
    for (id, level) in &stack.enchantments {
        item.enchant(id, (*level).max(0) as u32);
    }
    if let Some(potion) = &stack.potion {
        item = item.with_potion(potion);
    }
    if let Some(amplifier) = stack.ominous_amplifier.filter(|a| *a > 0) {
        item.data_mut().ominous_amplifier = Some(amplifier as u8);
    }
    item
}
impl Game<'_> {
    /// The rest of `NetherPortalBlock.randomTick` after a won roll: with a
    /// player close enough for spawning, a zombified piglin spawns on the block
    /// under the portal column when it is a valid spawn.
    pub(super) fn portal_spawn(&mut self, pos: [i32; 3]) {
        // ChunkMap.anyPlayerCloseEnoughForSpawning: a non-spectator within 128
        // blocks of the chunk centre.
        let centre = [
            f64::from((pos[0] >> 4) * 16 + 8),
            f64::from((pos[2] >> 4) * 16 + 8),
        ];
        let spectator = self.spectator();
        let Some(close) = self.player.as_ref().filter(|_| !spectator).map(|p| {
            let (dx, dz) = (p.position.x - centre[0], p.position.z - centre[1]);
            dx * dx + dz * dz < 16384.0
        }) else {
            return;
        };
        if !close {
            return;
        }
        let mut y = pos[1];
        while &*self.res.facts.get(self.world.block(pos[0], y, pos[2])).name
            == "minecraft:nether_portal"
        {
            y -= 1;
        }
        // isValidSpawn: a sturdy top face (portal frames emit no light).
        let below = self.res.facts.get(self.world.block(pos[0], y, pos[2]));
        if below.sturdy_full & (1 << 1) == 0 {
            return;
        }
        let at = [
            f64::from(pos[0]) + 0.5,
            f64::from(y + 1),
            f64::from(pos[2]) + 0.5,
        ];
        let _ = self.spawn_mob("minecraft:zombified_piglin", at, false);
    }
}
