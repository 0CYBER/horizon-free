//! The biome parameters and constants of custom uniform expressions (spec 3.4): `BIOME_<NAME>`,
//! `CAT_<NAME>` and `PPT_<NAME>`.
//!
//! Decisions (impl-plan.md, OQ-11):
//! - `biome` is the biome's index in the game's biome registry, which lists vanilla's biomes
//!   sorted by identifier; `BIOME_<NAME>` is the index of `minecraft:<name>`. Names from before
//!   the 1.18 renames map to the biome that replaced them. A `BIOME_` name that matches no biome
//!   reads -1, so it never equals the player's biome.
//! - `biome_category` comes from the biome's name (categories left the game in 1.19).

use std::collections::HashMap;

/// `CAT_*`, in the order of their values.
pub const CATEGORIES: [&str; 17] = [
    "NONE",
    "TAIGA",
    "EXTREME_HILLS",
    "JUNGLE",
    "MESA",
    "PLAINS",
    "SAVANNA",
    "ICY",
    "THE_END",
    "BEACH",
    "FOREST",
    "OCEAN",
    "DESERT",
    "RIVER",
    "SWAMP",
    "MUSHROOM",
    "NETHER",
];

/// Old biome names and the biomes that replaced them.
const LEGACY: &[(&str, &str)] = &[
    ("EXTREME_HILLS", "windswept_hills"),
    ("MOUNTAINS", "windswept_hills"),
    ("GRAVELLY_MOUNTAINS", "windswept_gravelly_hills"),
    ("WOODED_MOUNTAINS", "windswept_forest"),
    ("SHATTERED_SAVANNA", "windswept_savanna"),
    ("GIANT_TREE_TAIGA", "old_growth_pine_taiga"),
    ("GIANT_SPRUCE_TAIGA", "old_growth_spruce_taiga"),
    ("TALL_BIRCH_FOREST", "old_growth_birch_forest"),
    ("SNOWY_TUNDRA", "snowy_plains"),
    ("JUNGLE_EDGE", "sparse_jungle"),
    ("WOODED_BADLANDS_PLATEAU", "wooded_badlands"),
    ("STONE_SHORE", "stony_shore"),
    ("NETHER", "nether_wastes"),
];

/// The constants every pack sees: `CAT_*` and `PPT_*`, plus `BIOME_*` for `biomes` (the
/// registry's identifiers, in id order).
pub fn constants(biomes: &[&str]) -> HashMap<String, f64> {
    let mut out: HashMap<String, f64> = CATEGORIES
        .iter()
        .enumerate()
        .map(|(i, c)| (format!("CAT_{c}"), i as f64))
        .collect();
    for (i, p) in ["NONE", "RAIN", "SNOW"].iter().enumerate() {
        out.insert(format!("PPT_{p}"), i as f64);
    }
    let index = |name: &str| {
        biomes
            .iter()
            .position(|b| b.strip_prefix("minecraft:") == Some(name))
    };
    for (i, b) in biomes.iter().enumerate() {
        if let Some(name) = b.strip_prefix("minecraft:") {
            out.insert(format!("BIOME_{}", name.to_ascii_uppercase()), i as f64);
        }
    }
    for (old, new) in LEGACY {
        if let Some(i) = index(new) {
            out.entry(format!("BIOME_{old}")).or_insert(i as f64);
        }
    }
    out
}

/// `biome_category` of the biome `minecraft:<name>`.
pub fn category(name: &str) -> i32 {
    let name = name.strip_prefix("minecraft:").unwrap_or(name);
    let has = |words: &[&str]| words.iter().any(|w| name.contains(w));
    let cat = if name == "the_void" {
        "NONE"
    } else if name == "the_end" || name.starts_with("end_") || name == "small_end_islands" {
        "THE_END"
    } else if has(&["nether", "crimson", "warped", "basalt", "soul_sand"]) {
        "NETHER"
    } else if has(&["ocean"]) {
        "OCEAN"
    } else if has(&["river"]) {
        "RIVER"
    } else if has(&["beach", "shore"]) {
        "BEACH"
    } else if has(&["swamp"]) {
        "SWAMP"
    } else if has(&["mushroom"]) {
        "MUSHROOM"
    } else if has(&["desert"]) {
        "DESERT"
    } else if has(&["badlands"]) {
        "MESA"
    } else if has(&["savanna"]) {
        "SAVANNA"
    } else if has(&["jungle"]) {
        "JUNGLE"
    } else if has(&["taiga"]) {
        "TAIGA"
    } else if has(&["windswept", "peaks", "slopes", "meadow", "grove"]) && name != "cherry_grove" {
        "EXTREME_HILLS"
    } else if has(&["frozen", "snowy", "ice"]) {
        "ICY"
    } else if has(&["forest", "garden", "cherry"]) {
        "FOREST"
    } else if has(&["plains"]) {
        "PLAINS"
    } else {
        "NONE"
    };
    CATEGORIES.iter().position(|c| *c == cat).unwrap_or(0) as i32
}

/// `biome_precipitation`: none where the biome has none, else rain from temperature 0.15 up,
/// snow below it (spec 3.4).
pub fn precipitation(has_precipitation: bool, temperature: f32) -> i32 {
    match (has_precipitation, temperature >= 0.15) {
        (false, _) => 0,
        (true, true) => 1,
        (true, false) => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants_categories_and_precipitation() {
        let c = constants(&[
            "minecraft:badlands",
            "minecraft:pale_garden",
            "minecraft:windswept_hills",
        ]);
        assert_eq!(c["BIOME_PALE_GARDEN"], 1.0);
        assert_eq!(c["BIOME_EXTREME_HILLS"], 2.0);
        assert_eq!(c["CAT_NETHER"], 16.0);
        assert_eq!(c["PPT_SNOW"], 2.0);
        assert!(!c.contains_key("BIOME_SULFUR_CAVES"));
        assert_eq!(category("minecraft:pale_garden"), 10);
        assert_eq!(category("minecraft:deep_frozen_ocean"), 11);
        assert_eq!(category("minecraft:snowy_taiga"), 1);
        assert_eq!(category("minecraft:crimson_forest"), 16);
        assert_eq!(category("minecraft:cherry_grove"), 10);
        assert_eq!(precipitation(true, 0.0), 2);
        assert_eq!(precipitation(true, 0.8), 1);
        assert_eq!(precipitation(false, 2.0), 0);
    }
}
