//! Natural spawning rules: `SpawnPlacements` placement types and each
//! species' spawn predicate (`checkAnimalSpawnRules`, `checkMonsterSpawnRules`,
//! water animals, bats...), and `finalizeSpawn` (variants, colours, babies,
//! equipment).
use super::{Mob, MobWorld, Species, farm::Climate};

/// Biome facts finalizeSpawn and the spawn rules read.
#[derive(Clone, Copy, Debug, Default)]
pub struct BiomeFacts {
    pub climate: Climate,
    /// Biome temperature at the position (strays in cold biomes, snow foxes).
    pub temperature: f32,
    pub snowy: bool,
    /// `#minecraft:spawns_snow_foxes` (`Fox.Variant.byBiome`: snow, else red).
    pub snow_fox: bool,
    pub swamp: bool,
    pub ocean: bool,
    pub mushroom: bool,
    pub desert: bool,
    pub jungle: bool,
    /// Frog variant from `spawns_warm/cold_variant_frogs` (0 temperate, 1 warm, 2 cold).
    pub frog: u8,
    /// Wolf variant index (`tame::WOLF_VARIANTS`).
    pub wolf: u8,
    /// `MoonBrightnessCheck(>= 0.9)`: the full moon.
    pub full_moon: bool,
    /// `spawns_coral_variant_zombie_nautilus` (the warm ocean).
    pub coral_nautilus: bool,
    /// `#minecraft:spawns_white_rabbits` (`Rabbit.getRandomRabbitVariant`).
    pub white_rabbit: bool,
    /// `#minecraft:spawns_gold_rabbits` (`Rabbit.getRandomRabbitVariant`).
    pub gold_rabbit: bool,
}
/// `SpawnPlacements.isSpawnPositionOk` for the entity type `entity`, of species `species`.
pub fn placement_ok(entity: &str, species: Species, w: &MobWorld, x: i32, y: i32, z: i32) -> bool {
    super::spawn_rules::is_spawn_position_ok(entity, species, w, x, y, z)
}
/// `Spider.SpiderEffectsGroupData.setRandomEffect`: Speed on two of five
/// draws, then Strength, Regeneration and Invisibility on one each.
pub fn spider_random_effect(random: &mut impl horizon_core::random::RandomSource) -> &'static str {
    match random.next_int_bound(5).unwrap_or(0) {
        0 | 1 => "minecraft:speed",
        2 => "minecraft:strength",
        3 => "minecraft:regeneration",
        _ => "minecraft:invisibility",
    }
}
/// Mob.finalizeSpawn for natural, chunk-generation and summoned mobs.
/// `DifficultyInstance`: the local difficulty at a position.
#[derive(Clone, Copy, Debug)]
pub struct LocalDifficulty {
    /// The world difficulty id, 0 peaceful to 3 hard.
    pub base: u8,
    pub effective: f32,
    /// The world difficulty is normal.
    pub normal: bool,
    /// The world difficulty is hard.
    pub hard: bool,
    /// `Difficulty.getId`: 0 peaceful to 3 hard.
    pub id: u8,
}
impl From<LocalDifficulty> for mod_api::Difficulty {
    fn from(d: LocalDifficulty) -> Self {
        Self::new(d.base, d.effective)
    }
}
impl From<mod_api::Difficulty> for LocalDifficulty {
    /// A mod's difficulty; a base past hard is hard.
    fn from(d: mod_api::Difficulty) -> Self {
        let base = d.base.min(3);
        Self {
            base,
            effective: d.effective,
            normal: base == 2,
            hard: base == 3,
            id: base,
        }
    }
}
/// `DimensionType.MOON_BRIGHTNESS_PER_PHASE`.
pub const MOON_BRIGHTNESS: [f32; 8] = [1.0, 0.75, 0.5, 0.25, 0.0, 0.25, 0.5, 0.75];
impl LocalDifficulty {
    /// `calculateDifficulty` from the difficulty id (0 peaceful to 3 hard),
    /// the overworld clock, the chunk's inhabited time and the moon.
    pub fn new(base: u8, total: i64, inhabited: i64, moon: f32) -> Self {
        let hard = base == 3;
        let normal = base == 2;
        if base == 0 {
            return Self {
                base,
                effective: 0.0,
                normal,
                hard,
                id: base,
            };
        }
        let global = ((total as f32 - 72000.0) / 1_440_000.0).clamp(0.0, 1.0) * 0.25;
        let mut local =
            (inhabited as f32 / 3_600_000.0).clamp(0.0, 1.0) * if hard { 1.0 } else { 0.75 };
        local += (moon * 0.25).clamp(0.0, global);
        if base == 1 {
            local *= 0.5;
        }
        Self {
            base,
            effective: f32::from(base) * (0.75 + global + local),
            normal,
            hard,
            id: base,
        }
    }
    /// `getSpecialMultiplier`.
    pub fn special(&self) -> f32 {
        mod_api::Difficulty::from(*self).special_multiplier()
    }
}
/// `Mob.populateDefaultEquipmentSlots`: sometimes a set of armour, from the
/// head down, of one random material.
/// A baby from `Animal.spawnChildFromBreeding`: `getBreedOffspring` creates
/// it without `finalizeSpawn`, so it never ran `Mob.finalizeSpawn`. Undoes
/// that part of finalizeSpawn: no `random_spawn_bonus`, right-handed.
pub fn bred_offspring(m: &mut Mob) {
    use super::finalize::{Attr, remove_modifier};
    remove_modifier(m, Attr::FollowRange, "minecraft:random_spawn_bonus");
    m.extra.left_handed = false;
}
/// VillagerType.byBiome
pub fn villager_type(b: &BiomeFacts) -> u8 {
    if b.desert {
        1
    } else if b.jungle {
        2
    } else if b.snowy {
        4
    } else if b.swamp {
        5
    } else if b.climate == Climate::Warm {
        3
    } else if b.climate == Climate::Cold && b.temperature < 0.4 {
        6
    } else {
        0
    }
}
#[cfg(test)]
#[path = "spawn_group_variant_tests.rs"]
mod spawn_group_variant_tests;
#[cfg(test)]
pub(in crate::mobs) mod tests {
    use super::*;
    use crate::mobs::finalize::{Attr, has_modifier, value};

    pub(in crate::mobs) fn types() -> Option<crate::mobs::MobTypes> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("items.json").exists() {
            return None;
        }
        Some(crate::mobs::MobTypes::load(&root).unwrap())
    }

    struct Level {
        registries: crate::mobs::finalize::SpawnRegistries,
        enchantments: crate::enchant::Enchantments,
        items: crate::items::Items,
    }
    impl crate::mobs::finalize::FinalizeLevel for Level {
        fn biome(&self, _: [i32; 3]) -> String {
            "minecraft:plains".into()
        }
        fn biome_tag(&self, _: &str, _: [i32; 3]) -> bool {
            false
        }
        fn structures_with_piece_at(&self, _: [i32; 3]) -> Vec<String> {
            Vec::new()
        }
        fn moon_phase(&self, _: [i32; 3]) -> usize {
            0
        }
        fn spawning_monsters(&self) -> bool {
            true
        }
        fn no_collision(&self, _: horizon_core::geometry::Aabb) -> bool {
            true
        }
        fn free_chicken(&self, _: horizon_core::geometry::Aabb) -> Option<u32> {
            None
        }
        fn ride_chicken(&mut self, _: u32, _: u32) {}
        fn game_time(&self) -> i64 {
            0
        }
        fn halloween(&self) -> bool {
            false
        }
        fn create(&mut self, _: &str, _: crate::mobs::spawn_rules::Reason) -> Option<Mob> {
            None
        }
        fn spawn_data(&self) -> crate::mobs::finalize::SpawnData<'_> {
            crate::mobs::finalize::SpawnData {
                registries: &self.registries,
                enchantments: &self.enchantments,
                items: &self.items,
                camel_husk_size: (1.0, 1.0),
            }
        }
    }
    pub(in crate::mobs) fn run_finalize(m: &mut Mob, difficulty: LocalDifficulty) {
        run_finalize_in(m, difficulty, None);
    }
    /// [`run_finalize`] for a member of a pack: `group` is the `SpawnGroupData` the previous
    /// member returned (`None` for the first), and the result is what this one returns.
    pub(in crate::mobs) fn run_finalize_in(
        m: &mut Mob,
        difficulty: LocalDifficulty,
        group: Option<crate::mobs::finalize::Group>,
    ) -> Option<crate::mobs::finalize::Group> {
        static LEVEL: std::sync::OnceLock<std::sync::Mutex<Level>> = std::sync::OnceLock::new();
        let mut level = LEVEL
            .get_or_init(|| {
                let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../.local/horizon-client")
                    .join(horizon_core::VANILLA_VERSION);
                let defaults = horizon_world::item_defaults::read(&root).unwrap();
                std::sync::Mutex::new(Level {
                    registries: crate::mobs::finalize::SpawnRegistries::load(&root),
                    enchantments: crate::enchant::Enchantments::load(&root, &defaults).unwrap(),
                    items: crate::items::Items::tags_only(
                        &root,
                        &horizon_world::loot::PackOverlay::NONE,
                        &defaults,
                    )
                    .unwrap(),
                })
            })
            .lock()
            .unwrap();
        let mut random = m.random.clone();
        let mut cx = crate::mobs::finalize::Ctx {
            level: &mut *level,
            random: &mut random,
            difficulty,
            created: Vec::new(),
            jockey_rider: None,
        };
        cx.finalize(m, crate::mobs::spawn_rules::Reason::Natural, group)
    }

    fn spawn(types: &crate::mobs::MobTypes, kind: &str, id: u32) -> Mob {
        spawn_with(types, kind, id, &LocalDifficulty::new(3, 0, 0, 1.0))
    }
    fn spawn_with(
        types: &crate::mobs::MobTypes,
        kind: &str,
        id: u32,
        difficulty: &LocalDifficulty,
    ) -> Mob {
        let kind = std::sync::Arc::new(types.types.get(kind).unwrap().clone());
        let mut m = Mob::new(
            id,
            kind,
            horizon_core::geometry::Vec3::new(0.5, 64.0, 0.5),
            0.0,
        );
        run_finalize(&mut m, *difficulty);
        m
    }
    /// The four effects `Spider.SpiderEffectsGroupData.setRandomEffect` picks.
    const SPIDER_EFFECTS: [&str; 4] = [
        "minecraft:speed",
        "minecraft:strength",
        "minecraft:regeneration",
        "minecraft:invisibility",
    ];
    /// `Spider.finalizeSpawn`: on Hard, `0.1 * getSpecialMultiplier()` of
    /// spiders get one of those four for good (`MobEffectInstance(effect, -1)`).
    /// `/summon`, eggs, spawners and natural spawns all run it; Normal and
    /// CaveSpider's own `finalizeSpawn` never roll.
    #[test]
    fn runtime_spiders_roll_a_permanent_effect_on_hard() {
        let Some(types) = types() else { return };
        // Long played-in time: `getSpecialMultiplier` at its 1.0 maximum.
        let hard = LocalDifficulty::new(3, 1_512_000, 3_600_000, 1.0);
        assert_eq!(hard.special(), 1.0);
        let mut carried = 0;
        let mut kinds = [0usize; 4];
        for id in 1..=400 {
            let m = spawn_with(&types, "minecraft:spider", id, &hard);
            let effects = &m.extra.effects;
            assert!(effects.len() <= 1, "{id}: {effects:?}");
            for stored in effects {
                let (effect, duration, amplifier) = (stored.id, stored.duration, stored.amplifier);
                let i = SPIDER_EFFECTS
                    .iter()
                    .position(|e| *e == effect)
                    .unwrap_or_else(|| panic!("{id}: {effect}"));
                assert_eq!((duration, amplifier), (-1, 0), "{id}: {effect}");
                kinds[i] += 1;
                carried += 1;
            }
        }
        // 10 percent of 400 is 40.
        assert!((25..=60).contains(&carried), "{carried} of 400 spiders");
        // Vanilla's `nextInt(5)`: Speed on 0 or 1, then one each.
        assert!(kinds.iter().all(|n| *n > 0), "{kinds:?}");
        assert!(
            kinds[0] >= kinds[1] && kinds[0] >= kinds[2] && kinds[0] >= kinds[3],
            "{kinds:?}"
        );
        // Normal: `level.getDifficulty() == Difficulty.HARD` is false.
        let normal = LocalDifficulty::new(2, 1_512_000, 3_600_000, 1.0);
        for id in 1..=400 {
            let m = spawn_with(&types, "minecraft:spider", id, &normal);
            assert!(
                m.extra.effects.is_empty(),
                "normal {id}: {:?}",
                m.extra.effects
            );
        }
        // CaveSpider.finalizeSpawn returns the group data without rolling.
        for id in 1..=400 {
            let m = spawn_with(&types, "minecraft:cave_spider", id, &hard);
            assert!(
                m.extra.effects.is_empty(),
                "cave spider {id}: {:?}",
                m.extra.effects
            );
        }
    }
    /// #2783: `/summon`, eggs, spawners and natural spawns run
    /// `WitherSkeleton.finalizeSpawn`: attack damage base 4.0, not the
    /// default 2.0.
    #[test]
    fn runtime_wither_skeletons_hit_for_four() {
        let Some(types) = types() else { return };
        let w = spawn(&types, "minecraft:wither_skeleton", 1);
        assert_eq!(value(&w, Attr::AttackDamage), 4.0);
        assert_eq!(w.attrs.attack_damage, 4.0);
    }
    /// #2783: runtime spawns run `Mob.finalizeSpawn`: the follow range
    /// bonus and 5 percent left-handed mobs.
    #[test]
    fn runtime_spawns_run_the_mob_finalize_spawn() {
        let Some(types) = types() else { return };
        let mut left = 0;
        for id in 1..=400 {
            let m = spawn(&types, "minecraft:skeleton", id);
            assert!(has_modifier(
                &m,
                Attr::FollowRange,
                "minecraft:random_spawn_bonus"
            ));
            left += usize::from(m.extra.left_handed);
        }
        // 5 percent of 400 is 20.
        assert!((5..=40).contains(&left), "{left} of 400 left-handed");
        // A bred baby never ran it.
        let mut cow = spawn(&types, "minecraft:cow", 1);
        cow.extra.left_handed = true;
        bred_offspring(&mut cow);
        assert!(!has_modifier(
            &cow,
            Attr::FollowRange,
            "minecraft:random_spawn_bonus"
        ));
        assert!(!cow.extra.left_handed);
        assert_eq!(cow.attrs.follow_range, value(&cow, Attr::FollowRange));
        // CaveSpider.finalizeSpawn does not call super.
        let c = spawn(&types, "minecraft:cave_spider", 1);
        assert!(!has_modifier(
            &c,
            Attr::FollowRange,
            "minecraft:random_spawn_bonus"
        ));
    }
    /// #11651: unfinalized spawns (EntityType.create) skip finalizeSpawn's rolls.
    #[test]
    fn unfinalized_spawns_skip_finalize_rolls() {
        let Some(types) = types() else { return };
        let kind = std::sync::Arc::new(types.types.get("minecraft:zombie").unwrap().clone());
        let m = Mob::new(
            1,
            kind,
            horizon_core::geometry::Vec3::new(0.5, 64.0, 0.5),
            0.0,
        );
        assert!(!has_modifier(
            &m,
            Attr::FollowRange,
            "minecraft:random_spawn_bonus"
        ));
        assert!(!m.extra.left_handed);
        assert!(m.equipment_stacks()[5].is_none());
        assert!(m.equipment_stacks()[0].is_none());
    }
    /// `Zombie` / `AbstractSkeleton` halloween garb: 25% of empty heads, never
    /// on other mobs, and not when it is not Halloween.
    #[test]
    fn halloween_pumpkins_only_undead_on_halloween() {
        let Some(types) = types() else { return };
        let mut pumpkins = 0;
        let mut lanterns = 0;
        for id in 1..=400 {
            let mut m = spawn(&types, "minecraft:skeleton", id);
            m.set_equipment_stack(5, None);
            let mut random = m.random.clone();
            crate::mobs::finalize::halloween_head(&mut m, &mut random, true);
            match m.equipment_stacks()[5]
                .as_ref()
                .map(|stack| stack.item.as_ref())
            {
                Some("minecraft:jack_o_lantern") => {
                    pumpkins += 1;
                    lanterns += 1;
                    assert_eq!(crate::mobs::finalize::stored_drop_chance(&m, 5), 0.0);
                }
                Some("minecraft:carved_pumpkin") => {
                    pumpkins += 1;
                    assert_eq!(crate::mobs::finalize::stored_drop_chance(&m, 5), 0.0);
                }
                Some(other) => panic!("unexpected head {other}"),
                None => {}
            }
        }
        assert!(
            (70..=130).contains(&pumpkins),
            "{pumpkins} of 400 skeletons"
        );
        assert!(lanterns < pumpkins / 2, "{lanterns} lanterns of {pumpkins}");
        let mut m = spawn(&types, "minecraft:skeleton", 1);
        m.set_equipment_stack(5, None);
        let mut random = m.random.clone();
        crate::mobs::finalize::halloween_head(&mut m, &mut random, false);
        assert!(m.equipment_stacks()[5].is_none());
        let mut cow = spawn(&types, "minecraft:cow", 1);
        cow.set_equipment_stack(5, None);
        let mut random = cow.random.clone();
        crate::mobs::finalize::halloween_head(&mut cow, &mut random, true);
        assert!(cow.equipment_stacks()[5].is_none());
    }
}
