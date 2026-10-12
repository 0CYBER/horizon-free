//! Fixes of 1.17 and 1.18, Caves & Cliffs (keys 2679 to 2970): what worlds
//! saved by 1.16.5 to 1.18.1 need before the fixes of [`super::fixes_old`].
use super::dynamic::{as_str, empty_map, set, stream, string};
use super::schema::{Key, Ty, key};
use super::{Cx, FixFn};
use crate::nbt::{Compound, JavaString, Tag};

pub(super) fn register(out: &mut Vec<(Key, FixFn)>) {
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
