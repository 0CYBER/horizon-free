//! `Game::biome_facts` over what it reads, so a helper thread can ask too.
use horizon_client::mobs::{farm::Climate, spawning::BiomeFacts};

/// The facts of the biome at a position, for `finalizeSpawn` and the variant rules.
pub(in crate::game) fn biome_facts_in(
    world: &horizon_client::world::World,
    biomes: &horizon_client::biome::Biomes,
    tags: &horizon_client::mobs::MobData,
    x: i32,
    y: i32,
    z: i32,
) -> BiomeFacts {
    let Some(chunk) = world.chunks.get(&(x >> 4, z >> 4)) else {
        return BiomeFacts::default();
    };
    let b = biomes.get(chunk.biome(x, y, z));
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
        snow_fox: has("minecraft:spawns_snow_foxes"),
        swamp: b.name.contains("swamp"),
        ocean: has("minecraft:is_ocean"),
        mushroom: b.name == "minecraft:mushroom_fields",
        desert: b.name == "minecraft:desert",
        jungle: has("minecraft:is_jungle"),
        wolf: horizon_client::mobs::tame::wolf_variant_for(&b.name, &has),
        full_moon: (world.day_time / 24000).rem_euclid(8) == 0,
        frog: if has("minecraft:spawns_warm_variant_frogs") {
            1
        } else if has("minecraft:spawns_cold_variant_frogs") {
            2
        } else {
            0
        },
    }
}
