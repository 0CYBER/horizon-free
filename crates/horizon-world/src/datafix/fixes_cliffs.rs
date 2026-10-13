//! Fixes of 1.17 and 1.18, Caves & Cliffs (keys 2679 to 2970): what worlds
//! saved by 1.16.5 to 1.18.1 need before the fixes of [`super::fixes_old`].
use super::dynamic::{
    as_bool, as_byte_buffer, as_int, as_int_stream, as_long_stream, as_number, as_str, ceillog2,
    empty_map, remove, rename, set, stream, string,
};
use super::fixes_a::block_rename;
use super::fixes_b::advancements_rename;
use super::schema::{Key, Ty, key};
use super::{Cx, FixFn, renamer};
use crate::nbt::{Compound, JavaString, Tag};

pub(super) fn register(out: &mut Vec<(Key, FixFn)>) {
    out.push((key(2679, 0), cauldron_rename));
    out.push((key(2680, 0), |cx| {
        let t = &[("minecraft:grass_path", "minecraft:dirt_path")];
        cx.rename(Ty::ItemName, renamer(t));
    }));
    out.push((key(2680, 0), |cx| {
        block_rename(cx, &[("minecraft:grass_path", "minecraft:dirt_path")])
    }));
    out.push((key(2690, 0), |cx| cx.rename(Ty::ItemName, renamer(COPPER))));
    out.push((key(2690, 0), |cx| block_rename(cx, COPPER)));
    out.push((key(2691, 0), |cx| {
        cx.rename(Ty::ItemName, renamer(WAXED_COPPER))
    }));
    out.push((key(2691, 0), |cx| block_rename(cx, WAXED_COPPER)));
    out.push((key(2693, 0), |cx| {
        add_flag(cx, "has_increased_height_already", false)
    }));
    out.push((key(2696, 0), |cx| {
        cx.rename(Ty::ItemName, renamer(GRIMSTONE))
    }));
    out.push((key(2696, 0), |cx| block_rename(cx, GRIMSTONE)));
    out.push((key(2700, 0), |cx| {
        block_rename(
            cx,
            &[
                ("minecraft:cave_vines_head", "minecraft:cave_vines"),
                ("minecraft:cave_vines_body", "minecraft:cave_vines_plant"),
            ],
        )
    }));
    out.push((key(2701, 0), feature_pool_elements));
    out.push((key(2702, 0), arrow_pickup));
    out.push((key(2707, 0), |cx| {
        add_flag(cx, "has_increased_height_already", true)
    }));
    out.push((key(2710, 0), |cx| {
        stats_rename(cx, &[("minecraft:play_one_minute", "minecraft:play_time")])
    }));
    out.push((key(2717, 0), |cx| {
        cx.rename(
            Ty::ItemName,
            renamer(&[(
                "minecraft:azalea_leaves_flowers",
                "minecraft:flowering_azalea_leaves",
            )]),
        )
    }));
    out.push((key(2717, 0), |cx| {
        block_rename(
            cx,
            &[(
                "minecraft:azalea_leaves_flowers",
                "minecraft:flowering_azalea_leaves",
            )],
        )
    }));
    out.push((key(2825, 0), |cx| {
        add_flag(cx, "has_increased_height_already", false)
    }));
    out.push((key(2831, 0), spawner_data));
    out.push((key(2832, 0), world_gen_height_and_biome));
    out.push((key(2832, 0), chunk_height_and_biome));
    // 2833 and 2852: `WorldGenSettingsDisallowOldCustomWorldsFix` refuses a
    // world whose dimension has no type; it changes nothing.
    out.push((key(2838, 0), |cx| {
        cx.rename(Ty::Biome, renamer(CAVES_AND_CLIFFS))
    }));
    out.push((key(2841, 0), chunk_proto_tick_list));
    out.push((key(2842, 0), chunk_renames));
    out.push((key(2843, 0), overreaching_ticks));
    out.push((key(2843, 0), |cx| {
        cx.rename(
            Ty::Biome,
            renamer(&[("minecraft:deep_warm_ocean", "minecraft:warm_ocean")]),
        )
    }));
    out.push((key(2846, 0), |cx| {
        advancements_rename(cx, &|id| {
            renamer(&[
                (
                    "minecraft:husbandry/play_jukebox_in_meadows",
                    "minecraft:adventure/play_jukebox_in_meadows",
                ),
                (
                    "minecraft:adventure/caves_and_cliff",
                    "minecraft:adventure/fall_from_world_height",
                ),
                (
                    "minecraft:adventure/ride_strider_in_overworld_lava",
                    "minecraft:nether/ride_strider_in_overworld_lava",
                ),
            ])(id)
            .map(str::to_owned)
        })
    }));
    out.push((key(2967, 0), structure_settings_flatten));
    out.push((key(2970, 0), structures_become_configured));
}

// ------------------------------------------------------------------ 2967

/// `StructureSettingsFlattenFix`.
fn structure_settings_flatten(cx: &mut Cx) {
    cx.each(Ty::WorldGenSettings, |v| {
        let Some(Tag::Compound(dims)) = v.get_mut("dimensions") else {
            return;
        };
        for dim in dims.values_mut() {
            let Some(settings) = dim.get_mut("generator").and_then(|g| g.get_mut("settings"))
            else {
                continue;
            };
            if let Some(structures) = settings.get_mut("structures") {
                *structures = flatten_structures(structures);
            }
        }
    });
}

fn flatten_structures(input: &Tag) -> Tag {
    let mut structures = match input.get("structures") {
        Some(Tag::Compound(c)) => {
            let mut out = Compound::new();
            for (k, s) in c {
                let mut s = s.clone();
                set(&mut s, "type", string("minecraft:random_spread"));
                out.insert(k.clone(), s);
            }
            Tag::Compound(out)
        }
        _ => empty_map(),
    };
    if let Some(stronghold) = input.get("stronghold") {
        let mut stronghold = stronghold.clone();
        set(
            &mut stronghold,
            "type",
            string("minecraft:concentric_rings"),
        );
        set(&mut structures, "minecraft:stronghold", stronghold);
    }
    structures
}

// ------------------------------------------------------------------ 2970

struct Conversion {
    biomes: &'static [(&'static [&'static str], &'static str)],
    fallback: &'static str,
}

const fn trivial(fallback: &'static str) -> Conversion {
    Conversion {
        biomes: &[],
        fallback,
    }
}

fn conversion(key: &str) -> Option<Conversion> {
    Some(match key {
        "mineshaft" => Conversion {
            biomes: &[(
                &[
                    "minecraft:badlands",
                    "minecraft:eroded_badlands",
                    "minecraft:wooded_badlands",
                ],
                "minecraft:mineshaft_mesa",
            )],
            fallback: "minecraft:mineshaft",
        },
        "shipwreck" => Conversion {
            biomes: &[(
                &["minecraft:beach", "minecraft:snowy_beach"],
                "minecraft:shipwreck_beached",
            )],
            fallback: "minecraft:shipwreck",
        },
        "ocean_ruin" => Conversion {
            biomes: &[(
                &[
                    "minecraft:warm_ocean",
                    "minecraft:lukewarm_ocean",
                    "minecraft:deep_lukewarm_ocean",
                ],
                "minecraft:ocean_ruin_warm",
            )],
            fallback: "minecraft:ocean_ruin_cold",
        },
        "village" => Conversion {
            biomes: &[
                (&["minecraft:desert"], "minecraft:village_desert"),
                (&["minecraft:savanna"], "minecraft:village_savanna"),
                (&["minecraft:snowy_plains"], "minecraft:village_snowy"),
                (&["minecraft:taiga"], "minecraft:village_taiga"),
            ],
            fallback: "minecraft:village_plains",
        },
        "ruined_portal" => Conversion {
            biomes: &[
                (&["minecraft:desert"], "minecraft:ruined_portal_desert"),
                (
                    &[
                        "minecraft:badlands",
                        "minecraft:eroded_badlands",
                        "minecraft:wooded_badlands",
                        "minecraft:windswept_hills",
                        "minecraft:windswept_forest",
                        "minecraft:windswept_gravelly_hills",
                        "minecraft:savanna_plateau",
                        "minecraft:windswept_savanna",
                        "minecraft:stony_shore",
                        "minecraft:meadow",
                        "minecraft:frozen_peaks",
                        "minecraft:jagged_peaks",
                        "minecraft:stony_peaks",
                        "minecraft:snowy_slopes",
                    ],
                    "minecraft:ruined_portal_mountain",
                ),
                (
                    &[
                        "minecraft:bamboo_jungle",
                        "minecraft:jungle",
                        "minecraft:sparse_jungle",
                    ],
                    "minecraft:ruined_portal_jungle",
                ),
                (
                    &[
                        "minecraft:deep_frozen_ocean",
                        "minecraft:deep_cold_ocean",
                        "minecraft:deep_ocean",
                        "minecraft:deep_lukewarm_ocean",
                        "minecraft:frozen_ocean",
                        "minecraft:ocean",
                        "minecraft:cold_ocean",
                        "minecraft:lukewarm_ocean",
                        "minecraft:warm_ocean",
                    ],
                    "minecraft:ruined_portal_ocean",
                ),
            ],
            fallback: "minecraft:ruined_portal",
        },
        "pillager_outpost" => trivial("minecraft:pillager_outpost"),
        "mansion" => trivial("minecraft:mansion"),
        "jungle_pyramid" => trivial("minecraft:jungle_pyramid"),
        "desert_pyramid" => trivial("minecraft:desert_pyramid"),
        "igloo" => trivial("minecraft:igloo"),
        "swamp_hut" => trivial("minecraft:swamp_hut"),
        "stronghold" => trivial("minecraft:stronghold"),
        "monument" => trivial("minecraft:monument"),
        "fortress" => trivial("minecraft:fortress"),
        "endcity" => trivial("minecraft:end_city"),
        "buried_treasure" => trivial("minecraft:buried_treasure"),
        "nether_fossil" => trivial("minecraft:nether_fossil"),
        "bastion_remnant" => trivial("minecraft:bastion_remnant"),
        _ => return None,
    })
}

/// `StructuresBecomeConfiguredFix.findUpdatedStructureType`.
fn updated_structure(key: &JavaString, chunk: &Tag) -> Option<String> {
    let conv = conversion(&key.to_string_lossy().to_lowercase())?;
    if conv.biomes.is_empty() {
        return Some(conv.fallback.to_owned());
    }
    // `guessConfiguration`: palette biome counts per mapping, in first-seen
    // order; the most frequent wins, the first on ties.
    let mapping = |biome: &str| {
        conv.biomes
            .iter()
            .find(|(list, _)| list.contains(&biome))
            .map(|(_, to)| *to)
    };
    let mut matches: Vec<(&str, i32)> = Vec::new();
    for s in stream(chunk.get("sections")) {
        let palette = s.get("biomes").and_then(|b| b.get("palette")).cloned();
        for biome in stream(palette.as_ref()) {
            if let Some(m) = mapping(&as_str(Some(&biome)).unwrap_or_default()) {
                match matches.iter_mut().find(|(k, _)| *k == m) {
                    Some((_, n)) => *n += 1,
                    None => matches.push((m, 1)),
                }
            }
        }
    }
    let best = matches
        .iter()
        .fold(None::<(&str, i32)>, |best, &(k, n)| match best {
            Some((_, b)) if b >= n => best,
            _ => Some((k, n)),
        });
    Some(best.map_or(conv.fallback, |(k, _)| k).to_owned())
}

/// `StructuresBecomeConfiguredFix`.
fn structures_become_configured(cx: &mut Cx) {
    cx.each(Ty::Chunk, |chunk| {
        let Some(structures) = chunk.get("structures").cloned() else {
            return;
        };
        let mut structures = structures;
        if let Some(Tag::Compound(starts)) = structures.get("starts") {
            let mut out: Vec<(String, Tag)> = Vec::new();
            for (k, start) in starts {
                if as_str(start.get("id")).as_deref().unwrap_or("INVALID") == "INVALID" {
                    continue;
                }
                if let Some(new) = updated_structure(k, chunk)
                    && !out.iter().any(|(n, _)| *n == new)
                {
                    let mut start = start.clone();
                    set(&mut start, "id", string(&new));
                    out.push((new, start));
                }
            }
            let map = out
                .into_iter()
                .map(|(k, v)| (JavaString::from(k.as_str()), v));
            set(&mut structures, "starts", Tag::Compound(map.collect()));
        } else if structures.get("starts").is_some() {
            set(&mut structures, "starts", empty_map());
        }
        if let Some(refs) = structures.get("References") {
            let mut out: Vec<(String, Vec<Tag>)> = Vec::new();
            if let Tag::Compound(refs) = refs {
                for (k, list) in refs {
                    let list = stream(Some(list));
                    if list.is_empty() {
                        continue;
                    }
                    if let Some(new) = updated_structure(k, chunk) {
                        match out.iter_mut().find(|(n, _)| *n == new) {
                            Some((_, old)) => old.extend(list),
                            None => out.push((new, list)),
                        }
                    }
                }
            }
            let map = out.into_iter().map(|(k, v)| {
                let longs = v
                    .iter()
                    .map(|t| super::dynamic::as_long(Some(t), 0))
                    .collect();
                (JavaString::from(k.as_str()), Tag::LongArray(longs))
            });
            set(&mut structures, "References", Tag::Compound(map.collect()));
        }
        set(chunk, "structures", structures);
    });
}

// ------------------------------------------------------------------ 2679 to 2717

/// `CauldronRenameFix`.
fn cauldron_rename(cx: &mut Cx) {
    cx.each(Ty::BlockState, |v| {
        if as_str(v.get("Name")).as_deref() != Some("minecraft:cauldron") {
            return;
        }
        let level = v
            .get("Properties")
            .and_then(|p| as_str(p.get("level")))
            .unwrap_or_else(|| "0".into());
        if level == "0" {
            remove(v, "Properties");
        } else {
            set(v, "Name", string("minecraft:water_cauldron"));
        }
    });
}

const COPPER: &[(&str, &str)] = &[
    (
        "minecraft:weathered_copper_block",
        "minecraft:oxidized_copper_block",
    ),
    (
        "minecraft:semi_weathered_copper_block",
        "minecraft:weathered_copper_block",
    ),
    (
        "minecraft:lightly_weathered_copper_block",
        "minecraft:exposed_copper_block",
    ),
    (
        "minecraft:weathered_cut_copper",
        "minecraft:oxidized_cut_copper",
    ),
    (
        "minecraft:semi_weathered_cut_copper",
        "minecraft:weathered_cut_copper",
    ),
    (
        "minecraft:lightly_weathered_cut_copper",
        "minecraft:exposed_cut_copper",
    ),
    (
        "minecraft:weathered_cut_copper_stairs",
        "minecraft:oxidized_cut_copper_stairs",
    ),
    (
        "minecraft:semi_weathered_cut_copper_stairs",
        "minecraft:weathered_cut_copper_stairs",
    ),
    (
        "minecraft:lightly_weathered_cut_copper_stairs",
        "minecraft:exposed_cut_copper_stairs",
    ),
    (
        "minecraft:weathered_cut_copper_slab",
        "minecraft:oxidized_cut_copper_slab",
    ),
    (
        "minecraft:semi_weathered_cut_copper_slab",
        "minecraft:weathered_cut_copper_slab",
    ),
    (
        "minecraft:lightly_weathered_cut_copper_slab",
        "minecraft:exposed_cut_copper_slab",
    ),
    (
        "minecraft:waxed_semi_weathered_copper",
        "minecraft:waxed_weathered_copper",
    ),
    (
        "minecraft:waxed_lightly_weathered_copper",
        "minecraft:waxed_exposed_copper",
    ),
    (
        "minecraft:waxed_semi_weathered_cut_copper",
        "minecraft:waxed_weathered_cut_copper",
    ),
    (
        "minecraft:waxed_lightly_weathered_cut_copper",
        "minecraft:waxed_exposed_cut_copper",
    ),
    (
        "minecraft:waxed_semi_weathered_cut_copper_stairs",
        "minecraft:waxed_weathered_cut_copper_stairs",
    ),
    (
        "minecraft:waxed_lightly_weathered_cut_copper_stairs",
        "minecraft:waxed_exposed_cut_copper_stairs",
    ),
    (
        "minecraft:waxed_semi_weathered_cut_copper_slab",
        "minecraft:waxed_weathered_cut_copper_slab",
    ),
    (
        "minecraft:waxed_lightly_weathered_cut_copper_slab",
        "minecraft:waxed_exposed_cut_copper_slab",
    ),
];

const WAXED_COPPER: &[(&str, &str)] = &[
    ("minecraft:waxed_copper", "minecraft:waxed_copper_block"),
    (
        "minecraft:oxidized_copper_block",
        "minecraft:oxidized_copper",
    ),
    (
        "minecraft:weathered_copper_block",
        "minecraft:weathered_copper",
    ),
    ("minecraft:exposed_copper_block", "minecraft:exposed_copper"),
];

const GRIMSTONE: &[(&str, &str)] = &[
    ("minecraft:grimstone", "minecraft:deepslate"),
    (
        "minecraft:grimstone_slab",
        "minecraft:cobbled_deepslate_slab",
    ),
    (
        "minecraft:grimstone_stairs",
        "minecraft:cobbled_deepslate_stairs",
    ),
    (
        "minecraft:grimstone_wall",
        "minecraft:cobbled_deepslate_wall",
    ),
    (
        "minecraft:polished_grimstone",
        "minecraft:polished_deepslate",
    ),
    (
        "minecraft:polished_grimstone_slab",
        "minecraft:polished_deepslate_slab",
    ),
    (
        "minecraft:polished_grimstone_stairs",
        "minecraft:polished_deepslate_stairs",
    ),
    (
        "minecraft:polished_grimstone_wall",
        "minecraft:polished_deepslate_wall",
    ),
    ("minecraft:grimstone_tiles", "minecraft:deepslate_tiles"),
    (
        "minecraft:grimstone_tile_slab",
        "minecraft:deepslate_tile_slab",
    ),
    (
        "minecraft:grimstone_tile_stairs",
        "minecraft:deepslate_tile_stairs",
    ),
    (
        "minecraft:grimstone_tile_wall",
        "minecraft:deepslate_tile_wall",
    ),
    ("minecraft:grimstone_bricks", "minecraft:deepslate_bricks"),
    (
        "minecraft:grimstone_brick_slab",
        "minecraft:deepslate_brick_slab",
    ),
    (
        "minecraft:grimstone_brick_stairs",
        "minecraft:deepslate_brick_stairs",
    ),
    (
        "minecraft:grimstone_brick_wall",
        "minecraft:deepslate_brick_wall",
    ),
    (
        "minecraft:chiseled_grimstone",
        "minecraft:chiseled_deepslate",
    ),
];

/// `SavedDataFeaturePoolElementFix`: the feature of jigsaw pieces' feature pool
/// elements, from an inline configuration to a configured feature id.
fn feature_pool_elements(cx: &mut Cx) {
    cx.each(Ty::StructureFeature, |v| {
        let Some(children) = v.get("Children") else {
            return;
        };
        if !super::dynamic::is_list(Some(children)) {
            return;
        }
        let fixed = stream(Some(children))
            .into_iter()
            .map(|mut child| {
                let id = as_str(child.get("id")).unwrap_or_default();
                if !matches!(
                    id.as_str(),
                    "minecraft:jigsaw"
                        | "minecraft:nvi"
                        | "minecraft:pcp"
                        | "minecraft:bastionremnant"
                        | "minecraft:runtime"
                ) {
                    return child;
                }
                let element = child.get("pool_element");
                let is_feature = element
                    .and_then(|e| as_str(e.get("element_type")))
                    .as_deref()
                    == Some("minecraft:feature_pool_element");
                if is_feature
                    && let Some(f) = child
                        .get_mut("pool_element")
                        .and_then(|p| p.get_mut("feature"))
                    && let Some(new) = feature_replacement(f)
                {
                    *f = string(new);
                }
                child
            })
            .collect();
        set(v, "Children", Tag::List(fixed));
    });
}

fn path_str(v: &Tag, path: &[&str]) -> String {
    let mut cur = Some(v.clone());
    for p in path {
        cur = match (cur, p.strip_prefix('[').and_then(|s| s.strip_suffix(']'))) {
            (Some(c), Some(index)) => {
                let i: usize = index.parse().unwrap_or(usize::MAX);
                stream(Some(&c)).into_iter().nth(i)
            }
            (Some(c), None) => c.get(p).cloned(),
            (None, _) => None,
        };
    }
    as_str(cur.as_ref()).unwrap_or_default()
}

/// `SavedDataFeaturePoolElementFix.getReplacement`.
fn feature_replacement(value: &Tag) -> Option<&'static str> {
    let ty = path_str(value, &["type"]);
    let name = path_str(value, &["name"]);
    let provider = path_str(value, &["config", "state_provider", "type"]);
    let state = path_str(value, &["config", "state_provider", "state", "Name"]);
    let first = path_str(
        value,
        &["config", "state_provider", "entries", "[0]", "data", "Name"],
    );
    let foliage = path_str(value, &["config", "foliage_placer", "type"]);
    let leaves = path_str(value, &["config", "leaves_provider", "state", "Name"]);
    let feature = if !ty.is_empty() {
        ty
    } else if name.is_empty() {
        return None;
    } else if name == "minecraft:normal_tree" {
        "minecraft:tree".to_owned()
    } else {
        name
    };
    match feature.as_str() {
        "minecraft:random_patch" => {
            if provider == "minecraft:simple_state_provider" {
                match state.as_str() {
                    "minecraft:sweet_berry_bush" => return Some("minecraft:patch_berry_bush"),
                    "minecraft:cactus" => return Some("minecraft:patch_cactus"),
                    _ => {}
                }
            } else if provider == "minecraft:weighted_state_provider"
                && (first == "minecraft:grass" || first == "minecraft:fern")
            {
                return Some("minecraft:patch_taiga_grass");
            }
        }
        "minecraft:block_pile" => {
            if provider != "minecraft:simple_state_provider"
                && provider != "minecraft:rotated_block_provider"
            {
                if provider == "minecraft:weighted_state_provider" {
                    match first.as_str() {
                        "minecraft:packed_ice" | "minecraft:blue_ice" => {
                            return Some("minecraft:pile_ice");
                        }
                        "minecraft:jack_o_lantern" | "minecraft:pumpkin" => {
                            return Some("minecraft:pile_pumpkin");
                        }
                        _ => {}
                    }
                }
            } else {
                match state.as_str() {
                    "minecraft:hay_block" => return Some("minecraft:pile_hay"),
                    "minecraft:melon" => return Some("minecraft:pile_melon"),
                    "minecraft:snow" => return Some("minecraft:pile_snow"),
                    _ => {}
                }
            }
        }
        "minecraft:flower" => return Some("minecraft:flower_plain"),
        "minecraft:tree" => match foliage.as_str() {
            "minecraft:acacia_foliage_placer" => return Some("minecraft:acacia"),
            "minecraft:blob_foliage_placer" if leaves == "minecraft:oak_leaves" => {
                return Some("minecraft:oak");
            }
            "minecraft:pine_foliage_placer" => return Some("minecraft:pine"),
            "minecraft:spruce_foliage_placer" => return Some("minecraft:spruce"),
            _ => {}
        },
        _ => {}
    }
    None
}

/// `AbstractArrowPickupFix`.
fn arrow_pickup(cx: &mut Cx) {
    cx.each(Ty::Entity, |e| {
        let id = as_str(e.get("id")).map(|s| super::dynamic::namespaced(&s));
        if !matches!(
            id.as_deref(),
            Some("minecraft:arrow" | "minecraft:spectral_arrow" | "minecraft:trident")
        ) || e.get("pickup").is_some()
        {
            return;
        }
        let from_player = as_bool(e.get("player"), true);
        set(e, "pickup", Tag::Byte(i8::from(from_player)));
        remove(e, "player");
    });
}

/// `StatsRenameFix`: custom statistics and the objectives that count them.
pub(super) fn stats_rename(cx: &mut Cx, table: &[(&str, &'static str)]) {
    let r = renamer(table);
    cx.each(Ty::Stats, |v| {
        if let Some(Tag::Compound(custom)) = v
            .get_mut("stats")
            .and_then(|s| s.get_mut("minecraft:custom"))
        {
            let old = std::mem::take(custom);
            for (k, value) in old {
                let name = super::dynamic::namespaced(&k.to_string_lossy());
                let name = r(&name).map_or(name, str::to_owned);
                custom.insert(JavaString::from(name.as_str()), value);
            }
        }
    });
    cx.each(Ty::Objective, |v| {
        if let Some(ct) = v.get_mut("CriteriaType")
            && as_str(ct.get("type")).as_deref() == Some("minecraft:custom")
            && let Some(id) = as_str(ct.get("id"))
        {
            let id = super::dynamic::namespaced(&id);
            let new = r(&id).map_or(id, str::to_owned);
            set(ct, "id", string(&new));
        }
    });
}

// ------------------------------------------------------------------ 2825 to 2843

/// `AddFlagIfNotPresentFix` over the world generation settings.
pub(super) fn add_flag(cx: &mut Cx, flag: &str, value: bool) {
    cx.each(Ty::WorldGenSettings, |v| {
        if let Tag::Compound(_) = v
            && v.get(flag).is_none()
        {
            set(v, flag, Tag::Byte(i8::from(value)));
        }
    });
}

/// `SpawnerDataFix`: spawn data and potentials wrapped as weighted entries.
fn spawner_data(cx: &mut Cx) {
    use super::schema::flat::UNTYPED;
    cx.each(Ty::UntaggedSpawner, |v| {
        // Only what read as typed is wrapped.
        if v.get("SpawnData").is_some_and(|d| d.get(UNTYPED).is_none())
            && let Some(data) = remove(v, "SpawnData")
        {
            set(v, "SpawnData", Tag::compound([("entity", data)]));
        }
        let untyped = |entries: &Tag| {
            stream(Some(entries))
                .iter()
                .any(|e| e.get("Entity").is_some_and(|t| t.get(UNTYPED).is_some()))
        };
        if let Some(potentials) = v.get("SpawnPotentials").filter(|p| !untyped(p)).cloned() {
            let entries = stream(Some(&potentials))
                .into_iter()
                .map(|mut entry| {
                    let weight = entry
                        .get("Weight")
                        .and_then(as_number)
                        .map_or(1, |n| n.int_value());
                    let entity = remove(&mut entry, "Entity");
                    remove(&mut entry, "Weight");
                    if let Some(e) = entity {
                        set(&mut entry, "entity", e);
                    }
                    Tag::compound([("weight", Tag::Int(weight)), ("data", entry)])
                })
                .collect();
            set(v, "SpawnPotentials", Tag::List(entries));
        }
    });
}

/// `WorldGenSettingsHeightAndBiomeFix`.
fn world_gen_height_and_biome(cx: &mut Cx) {
    cx.each(Ty::WorldGenSettings, |v| {
        let flag = v.get("has_increased_height_already").cloned();
        let was_exp_snap = flag.is_none();
        let was_previously_increased = flag.as_ref().is_none_or(|f| as_bool(Some(f), true));
        remove(v, "has_increased_height_already");
        let Some(generator) = v
            .get_mut("dimensions")
            .and_then(|d| d.get_mut("minecraft:overworld"))
            .and_then(|o| o.get_mut("generator"))
        else {
            return;
        };
        match as_str(generator.get("type")).as_deref() {
            Some("minecraft:noise") => {
                let mut large = false;
                if let Some(source) = generator.get_mut("biome_source") {
                    let ty = as_str(source.get("type")).unwrap_or_default();
                    if ty == "minecraft:vanilla_layered"
                        || (was_exp_snap && ty == "minecraft:multi_noise")
                    {
                        large = as_bool(source.get("large_biomes"), false);
                        *source = Tag::compound([
                            ("preset", string("minecraft:overworld")),
                            ("type", string("minecraft:multi_noise")),
                        ]);
                    }
                }
                if large
                    && let Some(settings) = generator.get_mut("settings")
                    && as_str(Some(settings)).as_deref() == Some("minecraft:overworld")
                {
                    *settings = string("minecraft:large_biomes");
                }
            }
            Some("minecraft:flat") if !was_previously_increased => {
                if let Some(layers) = generator
                    .get_mut("settings")
                    .and_then(|s| s.get_mut("layers"))
                {
                    let air = Tag::compound([
                        ("height", Tag::Int(64)),
                        ("block", string("minecraft:air")),
                    ]);
                    let mut list = vec![air];
                    list.extend(stream(Some(layers)));
                    *layers = Tag::List(list);
                }
            }
            _ => {}
        }
    });
}

const STATUS_IS_OR_AFTER_SURFACE: &[&str] = &[
    "surface",
    "carvers",
    "liquid_carvers",
    "features",
    "light",
    "spawn",
    "heightmaps",
    "full",
];
const STATUS_IS_OR_AFTER_NOISE: &[&str] = &[
    "noise",
    "surface",
    "carvers",
    "liquid_carvers",
    "features",
    "light",
    "spawn",
    "heightmaps",
    "full",
];
const BLOCKS_BEFORE_FEATURE_STATUS: &[&str] = &[
    "minecraft:air",
    "minecraft:basalt",
    "minecraft:bedrock",
    "minecraft:blackstone",
    "minecraft:calcite",
    "minecraft:cave_air",
    "minecraft:coarse_dirt",
    "minecraft:crimson_nylium",
    "minecraft:dirt",
    "minecraft:end_stone",
    "minecraft:grass_block",
    "minecraft:gravel",
    "minecraft:ice",
    "minecraft:lava",
    "minecraft:mycelium",
    "minecraft:nether_wart_block",
    "minecraft:netherrack",
    "minecraft:orange_terracotta",
    "minecraft:packed_ice",
    "minecraft:podzol",
    "minecraft:powder_snow",
    "minecraft:red_sand",
    "minecraft:red_sandstone",
    "minecraft:sand",
    "minecraft:sandstone",
    "minecraft:snow_block",
    "minecraft:soul_sand",
    "minecraft:soul_soil",
    "minecraft:stone",
    "minecraft:terracotta",
    "minecraft:warped_nylium",
    "minecraft:warped_wart_block",
    "minecraft:water",
    "minecraft:white_terracotta",
];
const HEIGHTMAP_TYPES: &[&str] = &[
    "WORLD_SURFACE_WG",
    "WORLD_SURFACE",
    "WORLD_SURFACE_IGNORE_SNOW",
    "OCEAN_FLOOR_WG",
    "OCEAN_FLOOR",
    "MOTION_BLOCKING",
    "MOTION_BLOCKING_NO_LEAVES",
];

/// `ChunkHeightAndBiomeFix.BIOMES_BY_ID`.
fn biome_by_id(id: i32) -> &'static str {
    match id {
        0 => "minecraft:ocean",
        1 => "minecraft:plains",
        2 => "minecraft:desert",
        3 => "minecraft:mountains",
        4 => "minecraft:forest",
        5 => "minecraft:taiga",
        6 => "minecraft:swamp",
        7 => "minecraft:river",
        8 => "minecraft:nether_wastes",
        9 => "minecraft:the_end",
        10 => "minecraft:frozen_ocean",
        11 => "minecraft:frozen_river",
        12 => "minecraft:snowy_tundra",
        13 => "minecraft:snowy_mountains",
        14 => "minecraft:mushroom_fields",
        15 => "minecraft:mushroom_field_shore",
        16 => "minecraft:beach",
        17 => "minecraft:desert_hills",
        18 => "minecraft:wooded_hills",
        19 => "minecraft:taiga_hills",
        20 => "minecraft:mountain_edge",
        21 => "minecraft:jungle",
        22 => "minecraft:jungle_hills",
        23 => "minecraft:jungle_edge",
        24 => "minecraft:deep_ocean",
        25 => "minecraft:stone_shore",
        26 => "minecraft:snowy_beach",
        27 => "minecraft:birch_forest",
        28 => "minecraft:birch_forest_hills",
        29 => "minecraft:dark_forest",
        30 => "minecraft:snowy_taiga",
        31 => "minecraft:snowy_taiga_hills",
        32 => "minecraft:giant_tree_taiga",
        33 => "minecraft:giant_tree_taiga_hills",
        34 => "minecraft:wooded_mountains",
        35 => "minecraft:savanna",
        36 => "minecraft:savanna_plateau",
        37 => "minecraft:badlands",
        38 => "minecraft:wooded_badlands_plateau",
        39 => "minecraft:badlands_plateau",
        40 => "minecraft:small_end_islands",
        41 => "minecraft:end_midlands",
        42 => "minecraft:end_highlands",
        43 => "minecraft:end_barrens",
        44 => "minecraft:warm_ocean",
        45 => "minecraft:lukewarm_ocean",
        46 => "minecraft:cold_ocean",
        47 => "minecraft:deep_warm_ocean",
        48 => "minecraft:deep_lukewarm_ocean",
        49 => "minecraft:deep_cold_ocean",
        50 => "minecraft:deep_frozen_ocean",
        127 => "minecraft:the_void",
        129 => "minecraft:sunflower_plains",
        130 => "minecraft:desert_lakes",
        131 => "minecraft:gravelly_mountains",
        132 => "minecraft:flower_forest",
        133 => "minecraft:taiga_mountains",
        134 => "minecraft:swamp_hills",
        140 => "minecraft:ice_spikes",
        149 => "minecraft:modified_jungle",
        151 => "minecraft:modified_jungle_edge",
        155 => "minecraft:tall_birch_forest",
        156 => "minecraft:tall_birch_hills",
        157 => "minecraft:dark_forest_hills",
        158 => "minecraft:snowy_taiga_mountains",
        160 => "minecraft:giant_spruce_taiga",
        161 => "minecraft:giant_spruce_taiga_hills",
        162 => "minecraft:modified_gravelly_mountains",
        163 => "minecraft:shattered_savanna",
        164 => "minecraft:shattered_savanna_plateau",
        165 => "minecraft:eroded_badlands",
        166 => "minecraft:modified_wooded_badlands_plateau",
        167 => "minecraft:modified_badlands_plateau",
        168 => "minecraft:bamboo_jungle",
        169 => "minecraft:bamboo_jungle_hills",
        170 => "minecraft:soul_sand_valley",
        171 => "minecraft:crimson_forest",
        172 => "minecraft:warped_forest",
        173 => "minecraft:basalt_deltas",
        174 => "minecraft:dripstone_caves",
        175 => "minecraft:lush_caves",
        177 => "minecraft:meadow",
        178 => "minecraft:grove",
        179 => "minecraft:snowy_slopes",
        180 => "minecraft:snowcapped_peaks",
        181 => "minecraft:lofty_peaks",
        182 => "minecraft:stony_peaks",
        _ => "minecraft:plains",
    }
}

fn paletted(palette: Tag, data: Option<Tag>) -> Tag {
    let mut out = Tag::compound([("palette", palette)]);
    if let Some(d) = data {
        set(&mut out, "data", d);
    }
    out
}

/// `ChunkHeightAndBiomeFix.makeBiomeContainer`.
fn biome_container(source: impl Fn(usize) -> i32) -> Tag {
    let mut ids: Vec<i32> = Vec::new();
    for i in 0..64 {
        let id = source(i);
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    let palette = Tag::List(ids.iter().map(|&id| string(biome_by_id(id))).collect());
    let bits = ceillog2(ids.len()) as usize;
    if bits == 0 {
        return paletted(palette, None);
    }
    let per_long = 64 / bits;
    let mut storage = vec![0i64; 64_usize.div_ceil(per_long)];
    let (mut cell, mut bit) = (0, 0);
    for i in 0..64 {
        let index = ids.iter().position(|&x| x == source(i)).unwrap_or(0) as i64;
        storage[cell] |= index << bit;
        bit += bits;
        if bit + bits > 64 {
            cell += 1;
            bit = 0;
        }
    }
    paletted(palette, Some(Tag::LongArray(storage)))
}

/// `ChunkHeightAndBiomeFix.getBiomeContainers`: the containers and whether the
/// chunk was already saved with the increased height.
fn biome_containers(level: &Tag, overworld: bool, min_section: i32) -> (Vec<Tag>, bool) {
    let count = if overworld { 24 } else { 16 };
    let old = as_int_stream(level.get("Biomes"));
    let get = |old: &[i32], i: usize| old[i] & 0xFF;
    match old {
        Some(old) if old.len() == 1536 => {
            let containers = (0..24)
                .map(|y| biome_container(|i| get(&old, y * 64 + i)))
                .collect();
            (containers, true)
        }
        Some(old) if old.len() == 1024 => {
            let mut containers = vec![Tag::End; count];
            for y in 0..16 {
                containers[(y - min_section) as usize] =
                    biome_container(|i| get(&old, y as usize * 64 + i));
            }
            if overworld {
                let below = biome_container(|i| get(&old, i % 16));
                let above = biome_container(|i| get(&old, i % 16 + 1008));
                for c in containers.iter_mut().take(4) {
                    *c = below.clone();
                }
                for c in containers.iter_mut().skip(20) {
                    *c = above.clone();
                }
            }
            (containers, false)
        }
        _ => {
            let plains = paletted(Tag::List(vec![string("minecraft:plains")]), None);
            (vec![plains; count], false)
        }
    }
}

/// `ChunkProtoTickListFix.PoorMansPalettedContainer`.
struct PoorMansContainer {
    palette: Vec<Tag>,
    data: Vec<i64>,
}

impl PoorMansContainer {
    fn get(&self, x: usize, y: usize, z: usize) -> Option<&Tag> {
        match self.palette.len() {
            0 => None,
            1 => self.palette.first(),
            n => {
                let bits = ceillog2(n).max(4) as usize;
                let per_long = 64 / bits;
                let index = (((y << 4) | z) << 4) | x;
                let cell = index / per_long;
                let value = *self.data.get(cell)?;
                let bit = (index - cell * per_long) * bits;
                let i = ((value >> bit) & ((1i64 << bits) - 1)) as usize;
                self.palette.get(i)
            }
        }
    }
    fn of(container: &Tag) -> Self {
        Self {
            palette: stream(container.get("palette")),
            data: as_long_stream(container.get("data")).unwrap_or_default(),
        }
    }
}

fn block_name(state: Option<&Tag>, default: &str) -> String {
    state.map_or_else(
        || default.to_owned(),
        |s| as_str(s.get("Name")).unwrap_or_else(|| default.to_owned()),
    )
}

/// `ChunkHeightAndBiomeFix.makeOptimizedPalettedContainer`.
fn optimized_container(palette: &Tag, data: &Tag) -> Tag {
    let mut list = stream(Some(palette));
    if list.len() == 1 {
        return paletted(palette.clone(), None);
    }
    let bits = as_long_stream(Some(data)).map_or(0, |d| d.len() as i64) * 64 / 4096;
    let expected = i64::from(ceillog2(list.len()));
    if bits <= expected {
        return paletted(palette.clone(), Some(data.clone()));
    }
    let air = Tag::compound([("Name", string("minecraft:air"))]);
    let minimum = (1i64 << (bits - 1)) + 1;
    for _ in 0..(minimum - list.len() as i64) {
        list.push(air.clone());
    }
    paletted(Tag::List(list), Some(data.clone()))
}

/// `ChunkHeightAndBiomeFix`.
fn chunk_height_and_biome(cx: &mut Cx) {
    cx.each(Ty::Chunk, |chunk| {
        let context = chunk.get("__context");
        let dimension = context
            .and_then(|c| as_str(c.get("dimension")))
            .unwrap_or_default();
        let generator = context
            .and_then(|c| as_str(c.get("generator")))
            .unwrap_or_default();
        let Some(level) = chunk.get_mut("Level") else {
            return;
        };
        let overworld = dimension == "minecraft:overworld";
        let min_section = if overworld { -4 } else { 0 };
        let (biomes, was_increased) = biome_containers(level, overworld, min_section);
        let air = paletted(
            Tag::List(vec![Tag::compound([("Name", string("minecraft:air"))])]),
            None,
        );
        let mut blocks_in_chunk: Vec<String> = Vec::new();
        let mut bedrock: Option<PoorMansContainer> = None;
        if let Some(sections) = level.get("Sections").cloned() {
            let mut done: Vec<i32> = Vec::new();
            let mut out = Vec::new();
            for mut section in stream(Some(&sections)) {
                let y = as_int(section.get("Y"), 0);
                let container = match section.get("Palette") {
                    Some(palette) => {
                        for state in stream(Some(palette)) {
                            let name = block_name(Some(&state), "minecraft:air");
                            if !blocks_in_chunk.contains(&name) {
                                blocks_in_chunk.push(name);
                            }
                        }
                        match section.get("BlockStates") {
                            Some(data) => optimized_container(palette, data),
                            None => air.clone(),
                        }
                    }
                    None => air.clone(),
                };
                let index = y - min_section;
                if index >= 0 && (index as usize) < biomes.len() {
                    set(&mut section, "biomes", biomes[index as usize].clone());
                }
                if !done.contains(&y) {
                    done.push(y);
                }
                if section.get("Y").and_then(as_number).map(|n| n.int_value()) == Some(0) {
                    bedrock = Some(PoorMansContainer::of(&container));
                }
                set(&mut section, "block_states", container);
                remove(&mut section, "Palette");
                remove(&mut section, "BlockStates");
                out.push(section);
            }
            for (i, b) in biomes.iter().enumerate() {
                let y = i as i32 + min_section;
                if !done.contains(&y) {
                    done.push(y);
                    out.push(Tag::compound([
                        ("Y", Tag::Int(y)),
                        ("block_states", air.clone()),
                        ("biomes", b.clone()),
                    ]));
                }
            }
            set(level, "Sections", Tag::List(out));
        }
        if overworld {
            predict_status_before_surface(level, &mut blocks_in_chunk);
        }
        update_chunk_tag(
            level,
            overworld,
            was_increased,
            generator == "minecraft:noise",
            bedrock.as_ref(),
        );
    });
}

fn predict_status_before_surface(level: &mut Tag, blocks: &mut Vec<String>) {
    let Some(status) = level.get("Status") else {
        return;
    };
    let current = as_str(Some(status)).unwrap_or_else(|| "empty".into());
    if STATUS_IS_OR_AFTER_SURFACE.contains(&current.as_str()) {
        return;
    }
    blocks.retain(|b| b != "minecraft:air");
    let has_non_air = !blocks.is_empty();
    blocks.retain(|b| !BLOCKS_BEFORE_FEATURE_STATUS.contains(&b.as_str()));
    let has_feature_blocks = !blocks.is_empty();
    let new = if has_feature_blocks {
        Some("liquid_carvers")
    } else if current == "noise" || has_non_air {
        Some("noise")
    } else if current == "biomes" {
        Some("structure_references")
    } else {
        None
    };
    if let Some(new) = new {
        set(level, "Status", string(new));
    }
}

fn update_chunk_tag(
    level: &mut Tag,
    overworld: bool,
    was_increased: bool,
    blending: bool,
    bedrock: Option<&PoorMansContainer>,
) {
    remove(level, "Biomes");
    if !overworld {
        return update_carving_masks(level, 16, 0);
    }
    if was_increased {
        return update_carving_masks(level, 24, 0);
    }
    if let Some(Tag::Compound(maps)) = level.get_mut("Heightmaps") {
        for ty in HEIGHTMAP_TYPES {
            if let Some(h) = maps.get_mut(&super::dynamic::key(ty)) {
                let values = as_long_stream(Some(h)).unwrap_or_default();
                *h = Tag::LongArray(values.into_iter().map(fixed_heightmap_long).collect());
            }
        }
    }
    for key in ["LiquidsToBeTicked", "PostProcessing", "ToBeTicked"] {
        let mut list = stream(level.get(key));
        if list.len() != 24 {
            for _ in 0..4 {
                list.insert(0, Tag::List(Vec::new()));
                list.push(Tag::List(Vec::new()));
            }
            set(level, key, Tag::List(list));
        }
    }
    update_carving_masks(level, 24, 4);
    if let Some(upgrade) = level.get_mut("UpgradeData")
        && let Some(indices) = upgrade.get("Indices")
    {
        let mut shifted = Compound::new();
        if let Tag::Compound(c) = indices {
            for (k, v) in c {
                if let Ok(i) = k.to_string_lossy().parse::<i32>() {
                    shifted.insert(JavaString::from((i + 4).to_string().as_str()), v.clone());
                }
            }
        }
        set(upgrade, "Indices", Tag::Compound(shifted));
    }
    if !blending {
        return;
    }
    let Some(status) = level.get("Status").cloned() else {
        return;
    };
    let last = as_str(Some(&status)).unwrap_or_default();
    if last == "empty" {
        return;
    }
    set(
        level,
        "blending_data",
        Tag::compound([(
            "old_noise",
            Tag::Byte(i8::from(STATUS_IS_OR_AFTER_NOISE.contains(&last.as_str()))),
        )]),
    );
    let Some(container) = bedrock else {
        return;
    };
    let mut missing = [0u64; 4];
    let mut any_bedrock = last == "noise";
    for z in 0..16 {
        for x in 0..16 {
            let state = container.get(x, 0, z);
            let name = state.map(|s| as_str(s.get("Name")).unwrap_or_default());
            if name.as_deref() == Some("minecraft:air") {
                let bit = z * 16 + x;
                missing[bit / 64] |= 1 << (bit % 64);
            }
            any_bedrock |= name.as_deref() == Some("minecraft:bedrock");
        }
    }
    let cardinality: u32 = missing.iter().map(|w| w.count_ones()).sum();
    if any_bedrock && cardinality != 256 {
        let target = if last == "full" {
            string("heightmaps")
        } else {
            status.clone()
        };
        // `BitSet.toLongArray`: trailing zero words dropped.
        let used = missing.iter().rposition(|&w| w != 0).map_or(0, |i| i + 1);
        let words = missing[..used].iter().map(|&w| w as i64).collect();
        set(
            level,
            "below_zero_retrogen",
            Tag::compound([
                ("target_status", target),
                ("missing_bedrock", Tag::LongArray(words)),
            ]),
        );
        set(level, "Status", string("empty"));
    }
    set(level, "isLightOn", Tag::Byte(0));
}

fn fixed_heightmap_long(value: i64) -> i64 {
    let mut out = 0i64;
    let mut bit = 0;
    while bit + 9 <= 64 {
        let old = (value >> bit) & 511;
        let new = if old == 0 { 0 } else { (old + 64).min(511) };
        out |= new << bit;
        bit += 9;
    }
    out
}

fn update_carving_masks(level: &mut Tag, sections: usize, below: usize) {
    let mut masks = match level.get("CarvingMasks") {
        Some(m) => m.clone(),
        None => empty_map(),
    };
    if let Tag::Compound(c) = &mut masks {
        for v in c.values_mut() {
            let bytes = as_byte_buffer(Some(v)).unwrap_or_default();
            // `BitSet.valueOf(bytes).toLongArray()`: little-endian words,
            // trailing zero words dropped.
            let mut words: Vec<i64> = bytes
                .chunks(8)
                .map(|c| {
                    let mut b = [0u8; 8];
                    b[..c.len()].copy_from_slice(c);
                    i64::from_le_bytes(b)
                })
                .collect();
            while words.last() == Some(&0) {
                words.pop();
            }
            let mut out = vec![0i64; 64 * sections];
            out[64 * below..64 * below + words.len()].copy_from_slice(&words);
            *v = Tag::LongArray(out);
        }
    }
    // `updateMapValues` of something that is not a map leaves it unchanged.
    set(level, "CarvingMasks", masks);
}

fn liquid(state: Option<&Tag>) -> &'static str {
    let Some(state) = state else {
        return "minecraft:empty";
    };
    let block = as_str(state.get("Name")).unwrap_or_default();
    let props = state.get("Properties");
    let level = || as_int(props.and_then(|p| p.get("level")), 0);
    match block.as_str() {
        "minecraft:water" => {
            if level() == 0 {
                "minecraft:water"
            } else {
                "minecraft:flowing_water"
            }
        }
        "minecraft:lava" => {
            if level() == 0 {
                "minecraft:lava"
            } else {
                "minecraft:flowing_lava"
            }
        }
        _ => {
            let always = matches!(
                block.as_str(),
                "minecraft:bubble_column"
                    | "minecraft:kelp"
                    | "minecraft:kelp_plant"
                    | "minecraft:seagrass"
                    | "minecraft:tall_seagrass"
            );
            if always || as_bool(props.and_then(|p| p.get("waterlogged")), false) {
                "minecraft:water"
            } else {
                "minecraft:empty"
            }
        }
    }
}

/// `ChunkProtoTickListFix`.
fn chunk_proto_tick_list(cx: &mut Cx) {
    cx.each(Ty::Chunk, |chunk| {
        let Some(level) = chunk.get_mut("Level") else {
            return;
        };
        if let Some(t) = remove(level, "LiquidTicks") {
            set(level, "fluid_ticks", t);
        }
        let mut lowest = 0i32;
        let mut containers: Vec<(i32, PoorMansContainer)> = Vec::new();
        for section in stream(level.get("Sections")) {
            let y = as_int(section.get("Y"), i32::MAX);
            if y == i32::MAX {
                continue;
            }
            if section.get("biomes").is_some() {
                lowest = lowest.min(y);
            }
            if let Some(container) = section.get("block_states") {
                match containers.iter_mut().find(|(sy, _)| *sy == y) {
                    Some((_, c)) => *c = PoorMansContainer::of(container),
                    None => containers.push((y, PoorMansContainer::of(container))),
                }
            }
        }
        let min_y = lowest as i8;
        if level.get("yPos").is_some() {
            set(level, "yPos", Tag::Byte(min_y));
        }
        if level.get("TileTicks").is_some() || level.get("fluid_ticks").is_some() {
            return;
        }
        let x = as_int(level.get("xPos"), 0);
        let z = as_int(level.get("zPos"), 0);
        let make = |level: &Tag, key: &str, liquid_list: bool| {
            let mut out = Vec::new();
            for (index, positions) in stream(level.get(key)).iter().enumerate() {
                let section_y = index as i32 + i32::from(min_y);
                let container = containers
                    .iter()
                    .find(|(y, _)| *y == section_y)
                    .map(|(_, c)| c);
                for pos in stream(Some(positions)) {
                    let pos = as_number(&pos).map_or(-1, |n| i32::from(n.int_value() as i16));
                    if pos <= 0 {
                        continue;
                    }
                    let (rx, ry, rz) = (pos & 15, (pos >> 4) & 15, (pos >> 8) & 15);
                    let state =
                        container.and_then(|c| c.get(rx as usize, ry as usize, rz as usize));
                    let ty = if liquid_list {
                        liquid(state).to_owned()
                    } else {
                        block_name(state, "minecraft:air")
                    };
                    out.push(Tag::compound([
                        ("i", string(&ty)),
                        ("x", Tag::Int(x * 16 + rx)),
                        ("y", Tag::Int(section_y * 16 + ry)),
                        ("z", Tag::Int(z * 16 + rz)),
                        ("t", Tag::Int(0)),
                        ("p", Tag::Int(0)),
                    ]));
                }
            }
            Tag::List(out)
        };
        let fluid_ticks = make(level, "LiquidsToBeTicked", true);
        let block_ticks = make(level, "ToBeTicked", false);
        set(level, "TileTicks", block_ticks);
        remove(level, "ToBeTicked");
        remove(level, "LiquidsToBeTicked");
        set(level, "fluid_ticks", fluid_ticks);
    });
}

/// `ChunkRenamesFix`: the Level tag merged into the chunk, fields renamed.
fn chunk_renames(cx: &mut Cx) {
    cx.each(Ty::Chunk, |chunk| {
        let Some(Tag::Compound(level)) = remove(chunk, "Level") else {
            return;
        };
        for (k, v) in level {
            if let Tag::Compound(c) = chunk {
                c.insert(k, v);
            }
        }
        for (from, to) in [
            ("TileEntities", "block_entities"),
            ("TileTicks", "block_ticks"),
            ("Entities", "entities"),
            ("Sections", "sections"),
        ] {
            rename(chunk, from, to);
        }
        if let Some(s) = chunk.get_mut("Structures") {
            rename(s, "Starts", "starts");
        }
        rename(chunk, "Structures", "structures");
    });
}

/// `OverreachingTickFix`.
fn overreaching_ticks(cx: &mut Cx) {
    cx.each(Ty::Chunk, |chunk| {
        let x = as_int(chunk.get("xPos"), 0);
        let z = as_int(chunk.get("zPos"), 0);
        for (list, name) in [
            ("block_ticks", "neighbor_block_ticks"),
            ("fluid_ticks", "neighbor_fluid_ticks"),
        ] {
            let Some(ticks) = chunk.get(list) else {
                continue;
            };
            let over: Vec<Tag> = stream(Some(ticks))
                .into_iter()
                .filter(|t| {
                    let dx = (x - (as_int(t.get("x"), 0) >> 4)).abs();
                    let dz = (z - (as_int(t.get("z"), 0) >> 4)).abs();
                    (dx != 0 || dz != 0) && dx <= 1 && dz <= 1
                })
                .collect();
            if !over.is_empty() {
                let mut upgrade = chunk.get("UpgradeData").cloned().unwrap_or_else(empty_map);
                set(&mut upgrade, name, Tag::List(over));
                set(chunk, "UpgradeData", upgrade);
            }
        }
    });
}

/// `CavesAndCliffsRenames`.
const CAVES_AND_CLIFFS: &[(&str, &str)] = &[
    ("minecraft:badlands_plateau", "minecraft:badlands"),
    ("minecraft:bamboo_jungle_hills", "minecraft:bamboo_jungle"),
    ("minecraft:birch_forest_hills", "minecraft:birch_forest"),
    ("minecraft:dark_forest_hills", "minecraft:dark_forest"),
    ("minecraft:desert_hills", "minecraft:desert"),
    ("minecraft:desert_lakes", "minecraft:desert"),
    (
        "minecraft:giant_spruce_taiga_hills",
        "minecraft:old_growth_spruce_taiga",
    ),
    (
        "minecraft:giant_spruce_taiga",
        "minecraft:old_growth_spruce_taiga",
    ),
    (
        "minecraft:giant_tree_taiga_hills",
        "minecraft:old_growth_pine_taiga",
    ),
    (
        "minecraft:giant_tree_taiga",
        "minecraft:old_growth_pine_taiga",
    ),
    (
        "minecraft:gravelly_mountains",
        "minecraft:windswept_gravelly_hills",
    ),
    ("minecraft:jungle_edge", "minecraft:sparse_jungle"),
    ("minecraft:jungle_hills", "minecraft:jungle"),
    ("minecraft:modified_badlands_plateau", "minecraft:badlands"),
    (
        "minecraft:modified_gravelly_mountains",
        "minecraft:windswept_gravelly_hills",
    ),
    ("minecraft:modified_jungle_edge", "minecraft:sparse_jungle"),
    ("minecraft:modified_jungle", "minecraft:jungle"),
    (
        "minecraft:modified_wooded_badlands_plateau",
        "minecraft:wooded_badlands",
    ),
    ("minecraft:mountain_edge", "minecraft:windswept_hills"),
    ("minecraft:mountains", "minecraft:windswept_hills"),
    (
        "minecraft:mushroom_field_shore",
        "minecraft:mushroom_fields",
    ),
    ("minecraft:shattered_savanna", "minecraft:windswept_savanna"),
    (
        "minecraft:shattered_savanna_plateau",
        "minecraft:windswept_savanna",
    ),
    ("minecraft:snowy_mountains", "minecraft:snowy_plains"),
    ("minecraft:snowy_taiga_hills", "minecraft:snowy_taiga"),
    ("minecraft:snowy_taiga_mountains", "minecraft:snowy_taiga"),
    ("minecraft:snowy_tundra", "minecraft:snowy_plains"),
    ("minecraft:stone_shore", "minecraft:stony_shore"),
    ("minecraft:swamp_hills", "minecraft:swamp"),
    ("minecraft:taiga_hills", "minecraft:taiga"),
    ("minecraft:taiga_mountains", "minecraft:taiga"),
    (
        "minecraft:tall_birch_forest",
        "minecraft:old_growth_birch_forest",
    ),
    (
        "minecraft:tall_birch_hills",
        "minecraft:old_growth_birch_forest",
    ),
    (
        "minecraft:wooded_badlands_plateau",
        "minecraft:wooded_badlands",
    ),
    ("minecraft:wooded_hills", "minecraft:forest"),
    ("minecraft:wooded_mountains", "minecraft:windswept_forest"),
    ("minecraft:lofty_peaks", "minecraft:jagged_peaks"),
    ("minecraft:snowcapped_peaks", "minecraft:frozen_peaks"),
];
