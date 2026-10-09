//! Natural spawning rules: `SpawnPlacements` placement types and each
//! species' spawn predicate (`checkAnimalSpawnRules`, `checkMonsterSpawnRules`,
//! water animals, bats...), and `finalizeSpawn` (variants, colours, babies,
//! equipment).
use super::{Mob, MobWorld, Species, data, farm::Climate};
use crate::blocks::Fluid;
use horizon_core::random::RandomSource;

/// Biome facts finalizeSpawn and the spawn rules read.
#[derive(Clone, Copy, Debug, Default)]
pub struct BiomeFacts {
    pub climate: Climate,
    /// Biome temperature at the position (strays in cold biomes, snow foxes).
    pub temperature: f32,
    pub snowy: bool,
    pub swamp: bool,
    pub ocean: bool,
    pub mushroom: bool,
    pub desert: bool,
    pub jungle: bool,
    /// Frog variant from `spawns_warm/cold_variant_frogs` (0 temperate, 1 warm, 2 cold).
    pub frog: u8,
}
/// SpawnPlacementTypes for the species.
pub fn placement_ok(species: Species, w: &MobWorld, x: i32, y: i32, z: i32) -> bool {
    let id = format!("minecraft:{}", super::species::def(species).name);
    match w.data.placement(&id) {
        data::Placement::OnGround => {
            let below = w.data.state(w.world.block(x, y - 1, z));
            let valid = match species {
                Species::Ocelot | Species::Parrot => below.flags & data::VALID_SPAWN_LEAVES != 0,
                Species::PolarBear => below.flags & data::VALID_SPAWN_ICE != 0,
                _ => below.flags & data::VALID_SPAWN != 0,
            };
            valid && empty(w, x, y, z) && empty(w, x, y + 1, z)
        }
        data::Placement::InWater => {
            w.fluid(x, y, z) == Fluid::Water
                && w.data.state(w.world.block(x, y + 1, z)).flags & data::REDSTONE_CONDUCTOR == 0
        }
        data::Placement::InLava => w.fluid(x, y, z) == Fluid::Lava,
        data::Placement::NoRestrictions => true,
    }
}
fn empty(w: &MobWorld, x: i32, y: i32, z: i32) -> bool {
    w.data.state(w.world.block(x, y, z)).flags & data::VALID_EMPTY_SPAWN != 0
}
/// Monster.isDarkEnoughToSpawn (overworld: sky light vs random(32), block light 0).
pub fn dark_enough(w: &MobWorld, random: &mut impl RandomSource, x: i32, y: i32, z: i32) -> bool {
    let light = w.world.light(x, y, z);
    let (sky, block) = (i32::from(light >> 4), i32::from(light & 15));
    if sky > random.next_int_bound(32).unwrap_or(0) {
        return false;
    }
    if block > 0 {
        return false;
    }
    let darken = if w.thundering { 10 } else { w.sky_darken() };
    (sky - darken).max(0) <= random.next_int_bound(8).unwrap_or(0)
}
/// The species' checkSpawnRules for natural spawns.
pub fn spawn_rules_ok(
    species: Species,
    w: &MobWorld,
    random: &mut impl RandomSource,
    x: i32,
    y: i32,
    z: i32,
) -> bool {
    use Species as S;
    let below = w.block_name(x, y - 1, z).to_owned();
    let light = w.world.light(x, y, z);
    let raw = i32::from(light >> 4).max(i32::from(light & 15));
    match species {
        // Animal.checkAnimalSpawnRules: animals_spawnable_on below and bright.
        S::Pig
        | S::Cow
        | S::Sheep
        | S::Chicken
        | S::Horse
        | S::Donkey
        | S::Llama
        | S::Wolf
        | S::Cat
        | S::Fox
        | S::Rabbit
        | S::Goat
        | S::Panda
        | S::Armadillo
        | S::Mooshroom
        | S::Turtle
        | S::Frog
        | S::Bee => {
            let tag = match species {
                S::Mooshroom => "minecraft:mooshrooms_spawnable_on",
                S::Wolf => "minecraft:wolves_spawnable_on",
                S::Fox => "minecraft:foxes_spawnable_on",
                S::Rabbit => "minecraft:rabbits_spawnable_on",
                S::Goat => "minecraft:goats_spawnable_on",
                S::Armadillo => "minecraft:armadillo_spawnable_on",
                S::Turtle => "minecraft:turtles_spawnable_on",
                S::Frog => "minecraft:frogs_spawnable_on",
                _ => "minecraft:animals_spawnable_on",
            };
            w.has_tag(tag, &below) && raw > 8
        }
        S::PolarBear => {
            w.has_tag("minecraft:polar_bears_spawnable_on_alternate", &below)
                || w.has_tag("minecraft:animals_spawnable_on", &below) && raw > 8
        }
        S::Ocelot | S::Parrot => {
            (w.has_tag("minecraft:parrots_spawnable_on", &below)
                || below.contains("leaves")
                || below == "minecraft:grass_block")
                && raw > 8
        }
        S::Bat => {
            // Bat.checkBatSpawnRules: below sea level, dark.
            if y >= 63 {
                return false;
            }
            let block = i32::from(light & 15).max(i32::from(light >> 4));
            block <= random.next_int_bound(4).unwrap_or(0)
        }
        S::Slime => {
            // Swamp at night by moon brightness, or slime chunks below y 40.
            y < 40 && random.next_int_bound(10).unwrap_or(1) == 0
        }
        // GlowSquid.checkGlowSquidSpawnRules: deep, pitch dark water.
        S::GlowSquid => y <= 63 - 33 && light == 0 && w.block_name(x, y, z) == "minecraft:water",
        S::Squid | S::Cod | S::Salmon | S::TropicalFish | S::Pufferfish | S::Dolphin => {
            // WaterAnimal.checkSurfaceWaterAnimalSpawnRules: sea level - 13 ..= sea level.
            (50..=63).contains(&y)
                && w.fluid(x, y - 1, z) == Fluid::Water
                && w.block_name(x, y + 1, z) == "minecraft:water"
        }
        S::Axolotl => w.block_name(x, y - 1, z) == "minecraft:clay",
        S::Strider => w.fluid(x, y, z) == Fluid::Lava,
        S::Ghast
        | S::MagmaCube
        | S::ZombifiedPiglin
        | S::Piglin
        | S::Hoglin
        | S::Blaze
        | S::WitherSkeleton
        | S::Enderman
            if w.nether =>
        {
            !w.block_name(x, y - 1, z).contains("nether_wart_block") || species == S::Ghast
        }
        S::Husk | S::Stray => dark_enough(w, random, x, y, z) && w.can_see_sky(x, y, z),
        S::Drowned => w.fluid(x, y, z) == Fluid::Water && dark_enough(w, random, x, y, z),
        _ => {
            // Monster.checkMonsterSpawnRules
            !w.nether && dark_enough(w, random, x, y, z) || w.nether
        }
    }
}
/// Mob.finalizeSpawn for natural, chunk-generation and summoned mobs.
pub fn finalize(m: &mut Mob, biome: &BiomeFacts) {
    use Species as S;
    let climate_index = match biome.climate {
        Climate::Temperate => 0,
        Climate::Warm => 1,
        Climate::Cold => 2,
    };
    match m.species() {
        S::Pig | S::Cow | S::Chicken => m.extra.variant = climate_index,
        S::Sheep => m.extra.color = super::farm::sheep_color(biome.climate, &mut m.random),
        S::Mooshroom => m.extra.variant = 0,
        S::Frog => m.extra.variant = biome.frog,
        S::Zombie | S::Husk | S::Drowned | S::ZombieVillager => {
            // Zombie.finalizeSpawn: 5% babies (normal difficulty).
            if m.random.next_float() < 0.05 {
                m.extra.age = i32::MIN / 2;
            }
            if m.species() != S::Drowned && m.random.next_float() < 0.01 {
                let r = m.random.next_int_bound(3).unwrap_or(0);
                m.extra.main_hand = Some(
                    if r == 0 {
                        "minecraft:iron_sword"
                    } else {
                        "minecraft:iron_shovel"
                    }
                    .into(),
                );
            }
            if m.species() == S::Drowned && m.random.next_float() < 0.0625 {
                m.extra.main_hand = Some(
                    if m.random.next_int_bound(16).unwrap_or(1) < 10 {
                        "minecraft:trident"
                    } else {
                        "minecraft:fishing_rod"
                    }
                    .into(),
                );
            }
            if m.species() == S::ZombieVillager {
                m.extra.variant = villager_type(biome);
                m.extra.variant2 = m.random.next_int_bound(15).unwrap_or(0) as u8;
            }
        }
        S::Villager => {
            m.extra.variant = villager_type(biome);
        }
        S::TropicalFish => super::aquatic::finalize(m),
        _ => {}
    }
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
