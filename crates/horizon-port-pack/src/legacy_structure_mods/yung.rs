//! YUNG's API and the mods built on it: structures, placements, pool
//! elements and processors.
use crate::worldgen::{ENHANCED_TERRAIN, HORIZON_ENHANCED_TERRAIN, moogs_terrain, placement_rules};
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// YUNG's placements: vanilla's random spread with an enhanced exclusion
/// zone (several sets), plus, for the temples, no start near an ocean or a
/// river, and for strongholds, starts only in odd rings around the origin.
const YUNG_SPREADS: [&str; 4] = [
    "yungsapi:enhanced_random_spread",
    "betterdeserttemples:desert_temple",
    "betterjungletemples:jungle_temple",
    "betterstrongholds:stronghold",
];
/// YUNG's structures and the vanilla structure type each one's mod turns
/// off (its DisableVanilla*Mixin, on by default in each mod's config).
const YUNG_REPLACES: [(&str, &str); 7] = [
    (
        "betterdeserttemples:desert_temple",
        "minecraft:desert_pyramid",
    ),
    (
        "betterjungletemples:jungle_temple",
        "minecraft:jungle_temple",
    ),
    ("betterwitchhuts:witch_hut", "minecraft:swamp_hut"),
    (
        "betteroceanmonuments:ocean_monument",
        "minecraft:ocean_monument",
    ),
    ("betterstrongholds:stronghold", "minecraft:stronghold"),
    ("betterfortresses:fortress", "minecraft:fortress"),
    ("bettermineshafts:mineshaft_", "minecraft:mineshaft"),
];

/// A `structure` or `structure_set` file of YUNG's types as vanilla's: its
/// jigsaw, and its placements. `None` for any other file.
pub(crate) fn upgrade(
    kind: &str,
    v: &mut Value,
    set_tags: &BTreeMap<String, Vec<String>>,
    lost: &mut BTreeMap<String, usize>,
) -> Option<bool> {
    match kind {
        "structure" => match v["type"].as_str() {
            // Better Dungeons' small Nether dungeon is YUNG's jigsaw behind a
            // config switch that ships on.
            Some("yungsapi:yung_jigsaw" | "betterdungeons:small_nether_dungeon") => {
                yung_structure(v, lost);
                Some(true)
            }
            _ => None,
        },
        "structure_set" => match v["placement"]["type"].as_str() {
            Some(t) if YUNG_SPREADS.contains(&t) => {
                let t = t.to_owned();
                yung_placement(&t, &mut v["placement"], set_tags, lost);
                Some(true)
            }
            _ => None,
        },
        _ => None,
    }
}

/// A pool element of YUNG's type as vanilla's; whether it was one.
pub(crate) fn upgrade_pool_element(
    element: &mut Value,
    lost: &mut BTreeMap<String, usize>,
) -> bool {
    if element["element_type"] != "yungsapi:yung_single_element" {
        return false;
    }
    yung_element(element, lost);
    true
}

/// YUNG's API's jigsaw as vanilla's, laid out by its own assembler in the
/// game (`horizon:yungsapi`, `horizon_world::worldgen::structure::yung`):
/// the heights pieces stay within move under the key, its terrain
/// adaptation under `horizon:enhanced_terrain_adaptation`. A start offset in
/// the chunk is lost.
fn yung_structure(v: &mut Value, lost: &mut BTreeMap<String, usize>) {
    let Some(o) = v.as_object_mut() else {
        return;
    };
    o.insert("type".into(), "minecraft:jigsaw".into());
    let mut rules = serde_json::Map::new();
    for key in ["min_y", "max_y"] {
        if let Some(y) = o.remove(key) {
            rules.insert(key.into(), y);
        }
    }
    // YungJigsawStructure's adaptation (NONE by default): every piece's
    // junctions, and the start's box grown by its radius.
    let own = o.remove(ENHANCED_TERRAIN);
    let mut kernel = yung_terrain(own.as_ref().unwrap_or(&Value::Null), lost);
    kernel["rigid_junctions"] = true.into();
    kernel["inflate"] = true.into();
    o.insert(HORIZON_ENHANCED_TERRAIN.into(), kernel);
    for key in ["x_offset_in_chunk", "z_offset_in_chunk"] {
        if o.remove(key).is_some_and(|n| n != 0) {
            *lost.entry(key.into()).or_default() += 1;
        }
    }
    o.insert("horizon:yungsapi".into(), rules.into());
}

/// YUNG's single pool element as vanilla's, its rules under
/// `horizon:yungsapi`: name and `max_count`, depth range, priority,
/// `ignore_bounds`, its condition, dead-end pool and modifiers; its terrain
/// adaptation under `horizon:enhanced_terrain_adaptation`.
fn yung_element(element: &mut Value, lost: &mut BTreeMap<String, usize>) {
    let Some(o) = element.as_object_mut() else {
        return;
    };
    o.insert(
        "element_type".into(),
        "minecraft:single_pool_element".into(),
    );
    let mut rules = serde_json::Map::new();
    for (from, to) in [
        ("name", "name"),
        ("max_count", "max_count"),
        ("min_required_depth", "min_depth"),
        ("max_possible_depth", "max_depth"),
        ("is_priority", "priority"),
        ("ignore_bounds", "ignore_bounds"),
    ] {
        if let Some(x) = o.remove(from) {
            rules.insert(to.into(), x);
        }
    }
    if let Some(c) = o.remove("condition") {
        rules.insert("condition".into(), yung_condition(&c, lost));
    }
    // A piece's own adaptation, NONE included, replaces its structure's.
    if let Some(a) = o.remove(ENHANCED_TERRAIN) {
        o.insert(HORIZON_ENHANCED_TERRAIN.into(), yung_terrain(&a, lost));
    }
    if let Some(pool) = o.remove("deadend_pool").filter(Value::is_string) {
        rules.insert("deadend_pool".into(), pool);
    }
    let modifiers: Vec<Value> = o
        .remove("modifiers")
        .and_then(|m| m.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|m| yung_modifier(m, lost))
        .collect();
    if !modifiers.is_empty() {
        rules.insert("modifiers".into(), modifiers.into());
    }
    o.insert("horizon:yungsapi".into(), rules.into());
}

/// One of YUNG's structure modifiers in the game's terms: its condition and
/// its transform and delay_generation actions, on the piece itself (`self`,
/// the API's only target). Another target loses the modifier, another action
/// just that action.
fn yung_modifier(m: &Value, lost: &mut BTreeMap<String, usize>) -> Option<Value> {
    let target = m["target_selector"]["type"].as_str().unwrap_or_default();
    if target != "yungsapi:self" {
        *lost
            .entry(format!("yungsapi modifier target {target}"))
            .or_default() += 1;
        return None;
    }
    let mut actions = Vec::new();
    for a in m["actions"].as_array().into_iter().flatten() {
        let int = |k: &str| a[k].as_i64().unwrap_or(0);
        match a["type"].as_str().unwrap_or_default() {
            "yungsapi:transform" => actions.push(serde_json::json!({"type": "transform",
                "output": a["output"].clone(),
                "offset": [int("x_offset"), int("y_offset"), int("z_offset")]})),
            "yungsapi:delay_generation" => {
                actions.push(serde_json::json!({"type": "delay_generation"}))
            }
            other => {
                *lost
                    .entry(format!("yungsapi modifier action {other}"))
                    .or_default() += 1
            }
        }
    }
    let condition = match m.get("condition") {
        Some(c) => yung_condition(c, lost),
        None => serde_json::json!({"type": "true"}),
    };
    Some(serde_json::json!({"condition": condition, "actions": actions}))
}

/// YUNG's API's terrain adaptation as the game's kernel: its two presets
/// spelled out, NONE (also the default) as size 0. An aquifer override
/// (none of ATM10's) is lost.
fn yung_terrain(a: &Value, lost: &mut BTreeMap<String, usize>) -> Value {
    let kernel = |size: i32, distance: i32, top: &str, bottom: &str| {
        serde_json::json!({"kernel_size": size, "kernel_distance": distance,
            "top": top, "bottom": bottom})
    };
    match a["type"].as_str() {
        Some("yungsapi:carved_top_no_beard_large") => kernel(24, 16, "carve", "none"),
        Some("yungsapi:carved_top_no_beard_small") => kernel(12, 6, "carve", "none"),
        Some("yungsapi:custom") => {
            let mut out = serde_json::Map::new();
            for key in [
                "kernel_size",
                "kernel_distance",
                "top",
                "bottom",
                "bottom_offset",
                "padding",
            ] {
                if let Some(x) = a.get(key) {
                    out.insert(key.into(), x.clone());
                }
            }
            if a.get("aquifer_override")
                .is_some_and(|x| x["type"] != "yungsapi:none")
            {
                *lost.entry("yungsapi aquifer_override".into()).or_default() += 1;
            }
            moogs_terrain(&out.into(), lost).unwrap_or_else(|| kernel(0, 0, "none", "none"))
        }
        None | Some("yungsapi:none") => kernel(0, 0, "none", "none"),
        Some(other) => {
            *lost
                .entry(format!("yungsapi terrain adaptation {other}"))
                .or_default() += 1;
            kernel(0, 0, "none", "none")
        }
    }
}

/// One of YUNG's structure conditions in the game's terms. `mod_loaded`
/// holds for Minecraft alone: a mod that needs code is not ported, and the
/// pieces it guards would place its blocks. A condition the game does not
/// have does not hold, and is counted.
fn yung_condition(c: &Value, lost: &mut BTreeMap<String, usize>) -> Value {
    let t = c["type"].as_str().unwrap_or_default();
    let list = |lost: &mut BTreeMap<String, usize>| -> Value {
        let all = c["conditions"].as_array().into_iter().flatten();
        all.map(|x| yung_condition(x, lost))
            .collect::<Vec<_>>()
            .into()
    };
    let opt = |k: &str| c.get(k).cloned().unwrap_or(Value::Null);
    match t.trim_start_matches("yungsapi:") {
        "always_true" => serde_json::json!({"type": "true"}),
        "not" => {
            serde_json::json!({"type": "not", "condition": yung_condition(&c["condition"], lost)})
        }
        "all_of" => serde_json::json!({"type": "all_of", "conditions": list(lost)}),
        "any_of" => serde_json::json!({"type": "any_of", "conditions": list(lost)}),
        "depth" => serde_json::json!({"type": "depth",
            "min_depth": opt("min_required_depth"), "max_depth": opt("max_possible_depth")}),
        "altitude" => serde_json::json!({"type": "altitude",
            "bottom": opt("bottom_cutoff_y"), "top": opt("top_cutoff_y")}),
        "random_chance" => serde_json::json!({"type": "random_chance", "chance": opt("chance")}),
        "rotation" => serde_json::json!({"type": "rotation", "rotations": opt("rotations")}),
        "piece_in_horizontal_direction" => {
            // "yungsapi:all" (or no list) matches any piece.
            let pieces: Vec<Value> = c["pieces"].as_array().cloned().unwrap_or_default();
            let any = pieces.is_empty() || pieces.iter().any(|p| p == "yungsapi:all");
            serde_json::json!({"type": "piece_in_horizontal_direction",
                "pieces": if any { Vec::new() } else { pieces },
                "range": opt("range"), "rotation": opt("rotation")})
        }
        // The game reads "yungsapi:all" (or no list) as any piece.
        "piece_in_range" => serde_json::json!({"type": "piece_in_range",
            "pieces": c.get("pieces").cloned().unwrap_or_else(|| Vec::<Value>::new().into()),
            "horizontal_range": opt("horizontal_range"), "above_range": opt("above_range"),
            "below_range": opt("below_range")}),
        "mod_loaded" if c["modid"] == "minecraft" => serde_json::json!({"type": "true"}),
        "mod_loaded" => serde_json::json!({"type": "false"}),
        _ => {
            *lost.entry(format!("yungsapi condition {t}")).or_default() += 1;
            serde_json::json!({"type": "false"})
        }
    }
}

/// One of YUNG's placements as vanilla's random spread with `horizon:` keys:
/// the enhanced exclusion zone as `horizon:exclusion_zone`; the temples'
/// ocean and river search (findBiomeHorizontal around the chunk's middle
/// block at y 120, 48 blocks, every 2 quarts) as `horizon:avoid_biomes`; the
/// strongholds' rings as `horizon:ring_sections`.
fn yung_placement(
    t: &str,
    p: &mut Value,
    set_tags: &BTreeMap<String, Vec<String>>,
    lost: &mut BTreeMap<String, usize>,
) {
    placement_rules(p, set_tags, lost);
    p["type"] = "minecraft:random_spread".into();
    let Some(o) = p.as_object_mut() else {
        return;
    };
    match t {
        "betterdeserttemples:desert_temple" | "betterjungletemples:jungle_temple" => {
            o.insert(
                "horizon:avoid_biomes".into(),
                serde_json::json!({"biomes": ["#minecraft:is_river", "#minecraft:is_ocean"],
                    "radius": 48, "step": 2, "y": 120}),
            );
        }
        "betterstrongholds:stronghold" => {
            let mut rings = serde_json::Map::new();
            for (from, to) in [
                ("chunk_distance_to_first_ring", "distance_to_first_ring"),
                ("ring_chunk_thickness", "thickness"),
                ("max_ring_section", "max_section"),
            ] {
                if let Some(n) = o.remove(from) {
                    rings.insert(to.into(), n);
                }
            }
            o.insert("horizon:ring_sections".into(), rings.into());
        }
        _ => {}
    }
}

/// Marks a YUNG's structure that the game can start (its type is vanilla's
/// after the upgrade) with the vanilla type its mod turns off, under
/// `horizon:disables`. One the game cannot start leaves the vanilla
/// structure in place, so the world keeps one of the two.
pub(crate) fn disables(id: &str, v: &mut Value) -> bool {
    let Some((_, vanilla)) = YUNG_REPLACES.iter().find(|(s, _)| id.starts_with(s)) else {
        return false;
    };
    if !v["type"]
        .as_str()
        .is_some_and(|t| t.starts_with("minecraft:"))
    {
        return false;
    }
    v["horizon:disables"] = serde_json::json!([vanilla]);
    true
}

/// The game's type for the YUNG's processors that place blocks past their
/// template, which keep their fields (`horizon-world`'s `pillars.rs`).
pub(crate) fn pillar_type(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "betterfortresses:pillar_processor" | "betterjungletemples:pillar_processor" => {
            "horizon:yung_pillar"
        }
        _ => return None,
    })
}

/// The data form of one of YUNG's processors: the processors to put in its
/// place, or `None` when it has none (or is not YUNG's).
pub(crate) fn processor(kind: &str, p: &Value) -> Option<Vec<Value>> {
    let (ns, _) = kind.split_once(':')?;
    Some(match kind {
        // Better Fortresses' stair supports get the blocks its Java code
        // names.
        "betterfortresses:stair_pillar_processor" => {
            vec![json!({"processor_type": "horizon:stair_pillar",
            "blocks": {"minecraft:stone_brick_stairs": "minecraft:red_nether_brick_stairs",
                "minecraft:purpur_stairs": "minecraft:red_nether_bricks"},
            "front": "minecraft:red_nether_brick_stairs",
            "pillar": "minecraft:red_nether_bricks"})]
        }
        _ if ns == "betterdeserttemples"
            && let Some(processor) = desert_temple(kind) =>
        {
            vec![processor]
        }
        "betterdungeons:small_dungeon_chest_processor" => {
            // Its config's defaults: one chest at least, two at most, a
            // fifth of the others kept until then; ChestBlock covers both.
            vec![json!({"processor_type": "horizon:count_cap",
                "blocks": ["minecraft:chest", "minecraft:trapped_chest"],
                "min": 1, "max": 2, "chance": 0.2f32 as f64,
                "replacement": block("minecraft:cave_air")})]
        }
        "betterwitchhuts:leg_processor" => vec![yung_leg(
            "minecraft:brown_stained_glass",
            json!({"id": "minecraft:oak_log", "properties": {"axis": "y"}}),
            false,
        )],
        "betterwitchhuts:fence_leg_processor" => vec![yung_leg(
            "minecraft:crimson_fence",
            block("minecraft:oak_fence"),
            true,
        )],
        _ if ns == "betteroceanmonuments" => ocean_monument(kind)?,
        _ if ns == "betterdungeons" || ns == "betterdeserttemples" => vec![yung_spawner(kind, p)?],
        _ => return None,
    })
}

/// YUNG's leg processors (Better Ocean Monuments, Better Witch Huts): a
/// placeholder block becomes `state` and a column of it runs down through
/// air and fluids: the game's `horizon:yung_pillar` with fixed states, and
/// the placeholder's properties kept for the fence legs.
fn yung_leg(target: &str, state: Value, keep_properties: bool) -> Value {
    let fixed = json!({"entries": [], "defaultBlockState": state});
    json!({"processor_type": "horizon:yung_pillar", "target_block": block(target),
        "target_block_output": fixed, "pillar_states": fixed,
        "keep_properties": keep_properties})
}

/// Better Ocean Monuments' processors (no fields; what they do is in their
/// Java code): placeholder blocks as vanilla's `rule` or the game's
/// `horizon:pick` (a new `nextFloat` for each chance, as the mod draws),
/// and its sea-level ones as the game's `horizon:flood_air`,
/// `horizon:waterlog` and `horizon:keep_world_block`.
fn ocean_monument(kind: &str) -> Option<Vec<Value>> {
    let rule = |rules: &[(&str, Value)]| {
        let rules: Vec<Value> = rules
            .iter()
            .map(|(from, to)| {
                json!({"input_predicate": {"predicate_type": "minecraft:block_match", "block": from},
                "location_predicate": {"predicate_type": "minecraft:always_true"},
                "output_state": to})
            })
            .collect();
        json!({"processor_type": "minecraft:rule", "rules": rules})
    };
    let state = |id: &str, props: Value| json!({"id": id, "properties": props});
    let pick = |from: &str, picks: Vec<(f32, Value)>, default: Value, keep: bool| {
        let picks: Vec<Value> = picks
            .into_iter()
            .map(|(below, state)| json!({"below": below as f64, "state": state}))
            .collect();
        json!({"processor_type": "horizon:pick", "block": from, "picks": picks,
            "default": {"state": default}, "redraw": true, "keep_properties": keep})
    };
    let slabs = |from: &str, slab: &str, otherwise: &str| {
        let wet = |t: &str| state(slab, json!({"type": t, "waterlogged": "true"}));
        pick(
            from,
            vec![(0.4, wet("top")), (0.8, wet("bottom"))],
            block(otherwise),
            false,
        )
    };
    // Exposed below 0.1, else weathered below a new 0.3, else oxidized,
    // with the placeholder's properties for the stairs and slabs.
    let copper = |kind: &str| {
        let id = |age: &str| block(&format!("minecraft:{age}{kind}"));
        pick(
            &format!("minecraft:oxidized_{kind}"),
            vec![(0.1, id("exposed_")), (0.3, id("weathered_"))],
            id("oxidized_"),
            true,
        )
    };
    Some(match kind.strip_prefix("betteroceanmonuments:")? {
        "random_prismarine_slab_decoration_processor" => vec![slabs(
            "minecraft:lime_concrete",
            "minecraft:prismarine_slab",
            "minecraft:air",
        )],
        "random_dark_prismarine_slab_decoration_processor" => vec![slabs(
            "minecraft:blue_concrete",
            "minecraft:dark_prismarine_slab",
            "minecraft:water",
        )],
        "sand_gravel_processor" => {
            vec![rule(&[(
                "minecraft:yellow_stained_glass",
                block("minecraft:gravel"),
            )])]
        }
        "random_oxidization_processor" => [
            "copper",
            "cut_copper",
            "cut_copper_stairs",
            "cut_copper_slab",
        ]
        .into_iter()
        .map(copper)
        .collect(),
        "structure_void_processor" => vec![json!({"processor_type": "horizon:keep_world_block",
            "blocks": ["minecraft:purpur_slab", "minecraft:brown_wool"]})],
        "air_processor" => vec![json!({"processor_type": "horizon:flood_air"})],
        "waterlog_processor" => vec![json!({"processor_type": "horizon:waterlog"})],
        "seagrass_processor" => vec![rule(&[
            (
                "minecraft:red_sandstone_slab",
                state("minecraft:tall_seagrass", json!({"half": "lower"})),
            ),
            (
                "minecraft:end_stone_brick_slab",
                state("minecraft:tall_seagrass", json!({"half": "upper"})),
            ),
            ("minecraft:brick_slab", block("minecraft:seagrass")),
        ])],
        "random_sponge_processor" => vec![pick(
            "minecraft:orange_stained_glass",
            vec![(0.75, block("minecraft:wet_sponge"))],
            block("minecraft:water"),
            false,
        )],
        "leg_processor" => vec![yung_leg(
            "minecraft:blue_stained_glass",
            block("minecraft:prismarine_bricks"),
            false,
        )],
        _ => return None,
    })
}

/// A block state as 26.3 writes it.
fn block(id: &str) -> Value {
    json!({"id": id})
}

/// Better Desert Temples' placeholder blocks (glass, concrete and wool in
/// the templates) as the mod's processors turn them: the fixed ones as
/// vanilla's `rule`, the random ones as the game's `horizon:pick`, with the
/// mod's bounds on one `nextFloat` (YUNG's BlockStateRandomizer adds its
/// chances up in floats). The ones that read or change the world around
/// (pillars, Diorite's waterlogging) stay dropped.
fn desert_temple(kind: &str) -> Option<Value> {
    let swap = |from: &str, to: &str| {
        json!({"processor_type": "minecraft:rule", "rules": [{
            "input_predicate": {"predicate_type": "minecraft:block_match", "block": from},
            "location_predicate": {"predicate_type": "minecraft:always_true"},
            "output_state": block(to)}]})
    };
    let pick = |from: &str, picks: Vec<Value>, default: Value| json!({"processor_type": "horizon:pick", "block": from, "picks": picks, "default": default});
    // YUNG's BlockStateRandomizer: an entry wins below the sum of the
    // chances so far, summed as floats.
    let randomizer = |entries: &[(f32, &str)]| {
        let mut bottom = 0f32;
        entries
            .iter()
            .map(|(chance, id)| {
                bottom += chance;
                json!({"below": bottom as f64, "state": block(id)})
            })
            .collect::<Vec<_>>()
    };
    let at = |below: f32, id: &str| json!({"below": below as f64, "state": block(id)});
    // Sandstone stairs, upside down, facing Plane.HORIZONTAL.getRandomDirection.
    let stairs = || {
        let states: Vec<Value> = ["north", "east", "south", "west"]
            .iter()
            .map(|facing| {
                json!({"id": "minecraft:sandstone_stairs", "properties": {
                    "facing": facing, "half": "top", "shape": "straight", "waterlogged": "false"}})
            })
            .collect();
        json!({"states": states})
    };
    Some(match kind.strip_prefix("betterdeserttemples:")? {
        "blue_concrete_processor" => swap("minecraft:blue_concrete", "minecraft:blue_terracotta"),
        "yellow_concrete_processor" => {
            swap("minecraft:yellow_concrete", "minecraft:yellow_terracotta")
        }
        "red_wool_processor" => swap("minecraft:red_wool", "minecraft:tnt"),
        "white_stained_glass_processor" => pick(
            "minecraft:white_stained_glass",
            randomizer(&[(0.4, "minecraft:cobweb")]),
            json!({"state": block("minecraft:air")}),
        ),
        "polished_diorite_processor" => pick(
            "minecraft:polished_diorite",
            randomizer(&[(0.6, "minecraft:cut_sandstone")]),
            json!({"state": block("minecraft:sandstone")}),
        ),
        "orange_stained_glass_processor" => {
            let mut sus = at(0.01, "minecraft:suspicious_sand");
            sus["nbt"] = json!({"LootTable": "minecraft:archaeology/desert_pyramid"});
            let mut stairs = stairs();
            stairs["below"] = (0.45f32 as f64).into();
            pick(
                "minecraft:orange_stained_glass",
                vec![
                    sus,
                    at(0.1, "minecraft:sand"),
                    at(0.2, "minecraft:sandstone"),
                    stairs,
                ],
                json!({"state": block("minecraft:cut_sandstone")}),
            )
        }
        "red_stained_glass_processor" => pick(
            "minecraft:red_stained_glass",
            vec![at(0.3, "minecraft:sand"), at(0.75, "minecraft:sandstone")],
            stairs(),
        ),
        "pot_processor" => {
            // Four sherds, one float each; the pot back to its default state.
            let values: Vec<Value> = [
                (0.05f32, "archer"),
                (0.1, "miner"),
                (0.15, "prize"),
                (0.2, "skull"),
            ]
            .iter()
            .map(|(below, sherd)| {
                json!({"below": *below as f64, "value": format!("minecraft:{sherd}_pottery_sherd")})
            })
            .collect();
            pick(
                "minecraft:decorated_pot",
                Vec::new(),
                json!({"state": block("minecraft:decorated_pot"), "lists": [{
                    "key": "sherds", "count": 4, "values": values, "default": "minecraft:brick"}]}),
            )
        }
        _ => return None,
    })
}

/// An item as 26.3 saves it.
fn item(id: &str) -> Value {
    json!({"id": format!("minecraft:{id}"), "count": 1})
}

/// YUNG's processors that make a spawner of a fixed mob (YUNG's API's
/// `MobSpawnerData`), as vanilla's `rule`: every `input` block becomes a
/// spawner with that NBT.
fn yung_spawner(kind: &str, p: &Value) -> Option<Value> {
    const SPAWNER: &str = "minecraft:spawner";
    // (input block, mob, fields other than the defaults, whether the mob is
    // also the only potential, its equipment)
    let (input, mob, fields, potential, equipment): (_, _, &[(&str, i64)], _, _) = match kind {
        "betterdungeons:mob_spawner_processor" => {
            (SPAWNER, p["spawner_mob"].as_str()?, &[], true, None)
        }
        "betterdungeons:zombie_mob_spawner_processor" => (
            SPAWNER,
            "minecraft:zombie",
            &[("MaxNearbyEntities", 8)],
            true,
            None,
        ),
        "betterdungeons:skeleton_mob_spawner_processor" => (
            SPAWNER,
            "minecraft:skeleton",
            &[
                ("RequiredPlayerRange", 18),
                ("MaxNearbyEntities", 8),
                ("MaxSpawnDelay", 650),
            ],
            true,
            None,
        ),
        "betterdungeons:small_nether_dungeon_mob_spawner_processor" => {
            let mob = p["spawner_mob"].as_str()?;
            // Wither skeletons get netherite armor that never drops and a
            // stone sword (with the mod's default config, they and blazes
            // keep their loot).
            let armor = (mob == "minecraft:wither_skeleton").then(|| {
                json!({
                    "equipment": {
                        "feet": item("netherite_boots"),
                        "legs": item("netherite_leggings"),
                        "chest": item("netherite_chestplate"),
                        "head": item("netherite_helmet"),
                        "mainhand": item("stone_sword"),
                    },
                    "drop_chances": {"feet": 0.0, "legs": 0.0, "chest": 0.0, "head": 0.0},
                })
            });
            (SPAWNER, mob, &[], true, armor)
        }
        "betterdungeons:zombie_tombstone_spawner_processor" => (
            "minecraft:black_stained_glass",
            "minecraft:skeleton",
            &[],
            false,
            Some(json!({"equipment": {"mainhand": item("iron_sword")}})),
        ),
        "betterdeserttemples:gravel_processor" => (
            "minecraft:gravel",
            "minecraft:husk",
            &[("MaxNearbyEntities", 8), ("RequiredPlayerRange", 24)],
            false,
            None,
        ),
        // An empty hand list in 1.21.1: the skeletons spawn without a bow,
        // since a spawn entry with more than its id is not finalized.
        "betterdeserttemples:bone_block_processor" => (
            "minecraft:bone_block",
            "minecraft:skeleton",
            &[("RequiredPlayerRange", 32)],
            false,
            Some(json!({"equipment": {}})),
        ),
        "betterdeserttemples:infested_cracked_stone_bricks_processor" => (
            "minecraft:infested_cracked_stone_bricks",
            "minecraft:silverfish",
            &[("RequiredPlayerRange", 24)],
            false,
            None,
        ),
        // 1.21.1 ignores the items' old `Count` and `tag` (the shield's
        // pattern, the boots' dye): plain items.
        "betterdeserttemples:yellow_wool_processor" => (
            "minecraft:yellow_wool",
            "minecraft:husk",
            &[("MaxNearbyEntities", 8), ("RequiredPlayerRange", 24)],
            false,
            Some(json!({"equipment": {
                "mainhand": item("golden_sword"),
                "offhand": item("shield"),
                "feet": item("leather_boots"),
                "legs": item("leather_leggings"),
                "chest": item("golden_chestplate"),
                "head": item("chainmail_helmet"),
            }})),
        ),
        _ => return None,
    };
    let mut entity = json!({"id": mob});
    if let Some(Value::Object(extra)) = equipment {
        entity.as_object_mut()?.extend(extra);
    }
    let potentials = match potential {
        true => json!([{"data": {"entity": entity.clone()}, "weight": 1}]),
        false => json!([]),
    };
    let mut nbt = json!({
        "Delay": 20, "MinSpawnDelay": 200, "MaxSpawnDelay": 800, "SpawnCount": 4,
        "MaxNearbyEntities": 6, "RequiredPlayerRange": 16, "SpawnRange": 4,
        "SpawnData": {"entity": entity},
        "SpawnPotentials": potentials,
    });
    for (key, value) in fields {
        nbt[*key] = (*value).into();
    }
    Some(json!({
        "processor_type": "minecraft:rule",
        "rules": [{
            "input_predicate": {"predicate_type": "minecraft:block_match", "block": input},
            "location_predicate": {"predicate_type": "minecraft:always_true"},
            "output_state": {"Name": SPAWNER},
            "block_entity_modifier": {"type": "minecraft:append_static", "data": nbt},
        }],
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::legacy_structure_mods::data;
    use crate::processors::{ProcessorData, carry};
    use crate::worldgen::{Upgrade, upgrade};

    const NO_TAGS: BTreeMap<String, Vec<String>> = BTreeMap::new();

    #[test]
    fn yungs_jigsaws_keep_their_rules() {
        let mut report = Upgrade::default();
        let mut s = json!({"type": "yungsapi:yung_jigsaw", "start_pool": "y:start", "size": 20,
            "max_y": 60, "x_offset_in_chunk": 0, "max_distance_from_center": 80,
            "enhanced_terrain_adaptation": {"type": "yungsapi:custom", "kernel_size": 24,
                "kernel_distance": 24, "top": "carve", "bottom": "none"}});
        assert!(upgrade("structure", &mut s, &mut report, &NO_TAGS));
        assert_eq!(s["type"], "minecraft:jigsaw");
        assert_eq!(s["horizon:yungsapi"], json!({"max_y": 60}));
        assert_eq!(
            s["horizon:enhanced_terrain_adaptation"],
            json!({"kernel_size": 24, "kernel_distance": 24, "top": "carve", "bottom": "none",
                "rigid_junctions": true, "inflate": true})
        );
        for gone in ["max_y", "x_offset_in_chunk", "enhanced_terrain_adaptation"] {
            assert!(s.get(gone).is_none(), "{gone}");
        }
        // Better Dungeons' small Nether dungeon is the same jigsaw.
        let mut nether = json!({"type": "betterdungeons:small_nether_dungeon",
            "start_pool": "betterdungeons:small_nether_dungeon", "size": 10,
            "start_jigsaw_name": "betterdungeons:small_nether_dungeon_anchor",
            "max_distance_from_center": 80, "liquid_settings": "ignore_waterlogging"});
        assert!(upgrade("structure", &mut nether, &mut report, &NO_TAGS));
        assert_eq!(nether["type"], "minecraft:jigsaw");
        assert_eq!(nether["horizon:yungsapi"], json!({}));
        assert_eq!(
            nether["horizon:enhanced_terrain_adaptation"],
            json!({"kernel_size": 0, "kernel_distance": 0, "top": "none", "bottom": "none",
                "rigid_junctions": true, "inflate": true})
        );
        assert_eq!(
            nether["start_jigsaw_name"],
            "betterdungeons:small_nether_dungeon_anchor"
        );
        assert!(report.unsupported.is_empty());
        let mut pool = json!({"elements": [{"weight": 3, "element": {
            "element_type": "yungsapi:yung_single_element", "location": "y:room",
            "processors": "minecraft:empty", "projection": "rigid",
            "name": "room", "max_count": 2, "min_required_depth": 1, "is_priority": true,
            "ignore_bounds": false, "deadend_pool": "y:end",
            "modifiers": [
                {"condition": {"type": "yungsapi:piece_in_range", "pieces": ["y:pillar"],
                    "above_range": 40},
                 "target_selector": {"type": "yungsapi:self"},
                 "actions": [{"type": "yungsapi:transform", "output": ["y:covered"], "x_offset": -1},
                    {"type": "yungsapi:delay_generation"}, {"type": "yungsapi:spin"}]},
                {"condition": {"type": "yungsapi:always_true"},
                 "target_selector": {"type": "yungsapi:all"}, "actions": []}],
            "enhanced_terrain_adaptation": {"type": "yungsapi:carved_top_no_beard_small"},
            "condition": {"type": "yungsapi:all_of", "conditions": [
                {"type": "yungsapi:piece_in_horizontal_direction", "pieces": ["yungsapi:all"],
                    "range": 5, "rotation": "none"},
                {"type": "yungsapi:not", "condition": {"type": "yungsapi:mod_loaded", "modid": "create"}},
                {"type": "yungsapi:biome", "biomes": []}]}}}]});
        assert!(upgrade("template_pool", &mut pool, &mut report, &NO_TAGS));
        let e = &pool["elements"][0]["element"];
        assert_eq!(e["element_type"], "minecraft:single_pool_element");
        assert_eq!(e["location"], "y:room");
        assert_eq!(
            e["horizon:yungsapi"],
            json!({"name": "room", "max_count": 2, "min_depth": 1, "priority": true,
                "ignore_bounds": false, "condition": {"type": "all_of", "conditions": [
                    {"type": "piece_in_horizontal_direction", "pieces": [], "range": 5, "rotation": "none"},
                    {"type": "not", "condition": {"type": "false"}},
                    {"type": "false"}]},
                "deadend_pool": "y:end",
                "modifiers": [{"condition": {"type": "piece_in_range", "pieces": ["y:pillar"],
                        "horizontal_range": null, "above_range": 40, "below_range": null},
                    "actions": [{"type": "transform", "output": ["y:covered"], "offset": [-1, 0, 0]},
                        {"type": "delay_generation"}]}]})
        );
        assert!(e.get("deadend_pool").is_none() && e.get("modifiers").is_none());
        assert_eq!(
            e["horizon:enhanced_terrain_adaptation"],
            json!({"kernel_size": 12, "kernel_distance": 6, "top": "carve", "bottom": "none"})
        );
        assert_eq!(
            report.rules_lost,
            BTreeMap::from([
                ("yungsapi condition yungsapi:biome".into(), 1),
                ("yungsapi modifier action yungsapi:spin".into(), 1),
                ("yungsapi modifier target yungsapi:all".into(), 1),
            ])
        );
        assert!(report.unsupported.is_empty());
    }

    #[test]
    fn yungs_placements_keep_their_rules() {
        let tags = BTreeMap::from([(
            "bf:avoid".to_owned(),
            vec!["minecraft:nether_complexes".to_owned()],
        )]);
        let mut report = Upgrade::default();
        let mut fortress = json!({"placement": {"type": "yungsapi:enhanced_random_spread",
            "salt": 1, "spacing": 30, "separation": 20,
            "enhanced_exclusion_zone": {"other_set": "#bf:avoid", "chunk_count": 10}}});
        assert!(upgrade("structure_set", &mut fortress, &mut report, &tags));
        assert_eq!(
            fortress["placement"],
            json!({"type": "minecraft:random_spread", "salt": 1, "spacing": 30, "separation": 20,
                "horizon:exclusion_zone": {"other_sets": ["minecraft:nether_complexes"], "chunk_count": 10}})
        );
        let mut temple = json!({"placement": {"type": "betterdeserttemples:desert_temple",
            "salt": 2, "spacing": 30, "separation": 20,
            "exclusion_zone": {"other_set": "minecraft:villages", "chunk_count": 6}}});
        assert!(upgrade("structure_set", &mut temple, &mut report, &tags));
        assert_eq!(temple["placement"]["type"], "minecraft:random_spread");
        assert_eq!(temple["placement"]["exclusion_zone"]["chunk_count"], 6);
        assert_eq!(
            temple["placement"]["horizon:avoid_biomes"],
            json!({"biomes": ["#minecraft:is_river", "#minecraft:is_ocean"],
                "radius": 48, "step": 2, "y": 120})
        );
        let mut stronghold = json!({"placement": {"type": "betterstrongholds:stronghold",
            "salt": 3, "spacing": 85, "separation": 50,
            "chunk_distance_to_first_ring": 80, "ring_chunk_thickness": 96}});
        assert!(upgrade(
            "structure_set",
            &mut stronghold,
            &mut report,
            &tags
        ));
        assert_eq!(
            stronghold["placement"],
            json!({"type": "minecraft:random_spread", "salt": 3, "spacing": 85, "separation": 50,
                "horizon:ring_sections": {"distance_to_first_ring": 80, "thickness": 96}})
        );
        assert_eq!(report.mapped, 3);
        assert!(report.unsupported.is_empty());
        // The vanilla counterpart is turned off only for a structure the game starts.
        let mut ported = json!({"type": "minecraft:jigsaw"});
        assert!(disables("betterstrongholds:stronghold", &mut ported));
        assert_eq!(ported["horizon:disables"], json!(["minecraft:stronghold"]));
        let mut java = json!({"type": "bettermineshafts:mineshaft"});
        assert!(!disables("bettermineshafts:mineshaft_oak", &mut java));
        let mut circle = json!({"type": "minecraft:jigsaw"});
        assert!(!disables("betterwitchhuts:witch_circle", &mut circle));
    }

    #[test]
    fn yungs_spawners_become_rules() {
        let mut pool = json!({"elements": [{"element": {"processors": {"processors": [
            {"processor_type": "betterdungeons:skeleton_mob_spawner_processor"},
            {"processor_type": "betterdeserttemples:bone_block_processor"},
            {"processor_type": "betterdungeons:small_nether_dungeon_mob_spawner_processor",
             "spawner_mob": "minecraft:wither_skeleton"},
            {"processor_type": "betterdungeons:candle_processor"}]}}}]});
        let mut carried = BTreeMap::new();
        assert!(carry("template_pool", &mut pool, &data(), &mut carried));
        let p = &pool["elements"][0]["element"]["processors"]["processors"];
        let rule = |i: usize| &p[i]["rules"][0];
        assert_eq!(rule(0)["input_predicate"]["block"], "minecraft:spawner");
        assert_eq!(
            rule(0)["output_state"],
            json!({"Name": "minecraft:spawner"})
        );
        let nbt = &rule(0)["block_entity_modifier"]["data"];
        assert_eq!(
            (
                nbt["RequiredPlayerRange"].clone(),
                nbt["MaxNearbyEntities"].clone()
            ),
            (json!(18), json!(8))
        );
        assert_eq!(nbt["MaxSpawnDelay"], 650);
        assert_eq!(
            nbt["SpawnPotentials"],
            json!([{"data": {"entity": {"id": "minecraft:skeleton"}}, "weight": 1}])
        );
        // Bone blocks become bowless skeleton spawners with no potentials.
        let bone = &rule(1)["block_entity_modifier"]["data"];
        assert_eq!(rule(1)["input_predicate"]["block"], "minecraft:bone_block");
        assert_eq!(
            bone["SpawnData"],
            json!({"entity": {"id": "minecraft:skeleton", "equipment": {}}})
        );
        assert_eq!(bone["SpawnPotentials"], json!([]));
        let wither = &rule(2)["block_entity_modifier"]["data"]["SpawnData"]["entity"];
        assert_eq!(
            wither["equipment"]["head"]["id"],
            "minecraft:netherite_helmet"
        );
        assert_eq!(wither["drop_chances"]["feet"], 0.0);
        assert_eq!(p[3]["processor_type"], "betterdungeons:candle_processor");
        assert_eq!(carried.values().sum::<usize>(), 3);
    }

    #[test]
    fn desert_temple_placeholders_become_rules_and_picks() {
        let mut list = json!({"processors": [
            {"processor_type": "betterdeserttemples:red_wool_processor"},
            {"processor_type": "betterdeserttemples:white_stained_glass_processor"},
            {"processor_type": "betterdeserttemples:orange_stained_glass_processor"},
            {"processor_type": "betterdeserttemples:pot_processor"},
            {"processor_type": "betterdeserttemples:quartz_pillar_processor"}]});
        let mut carried = BTreeMap::new();
        assert!(carry("processor_list", &mut list, &data(), &mut carried));
        let p = list["processors"].as_array().unwrap();
        assert_eq!(
            p[0]["rules"][0]["output_state"],
            json!({"id": "minecraft:tnt"})
        );
        assert_eq!(p[1]["processor_type"], "horizon:pick");
        assert_eq!(p[1]["picks"][0]["below"], 0.4f32 as f64);
        assert_eq!(p[1]["default"]["state"]["id"], "minecraft:air");
        let glass = &p[2]["picks"];
        assert_eq!(glass[0]["state"]["id"], "minecraft:suspicious_sand");
        assert_eq!(
            glass[0]["nbt"]["LootTable"],
            "minecraft:archaeology/desert_pyramid"
        );
        assert_eq!(glass[3]["states"].as_array().unwrap().len(), 4);
        assert_eq!(glass[3]["below"], 0.45f32 as f64);
        assert_eq!(p[3]["default"]["lists"][0]["count"], 4);
        // The pillar reads the world below: still Java, still dropped later.
        assert_eq!(
            p[4]["processor_type"],
            "betterdeserttemples:quartz_pillar_processor"
        );
        assert_eq!(carried.values().sum::<usize>(), 4);
    }

    #[test]
    fn small_dungeon_chests_are_capped() {
        let mut list = json!({"processors": [
            {"processor_type": "betterdungeons:small_dungeon_chest_processor"}]});
        let mut carried = BTreeMap::new();
        assert!(carry("processor_list", &mut list, &data(), &mut carried));
        let cap = &list["processors"][0];
        assert_eq!(cap["processor_type"], "horizon:count_cap");
        assert_eq!(
            (cap["min"].as_u64(), cap["max"].as_u64()),
            (Some(1), Some(2))
        );
        assert_eq!(cap["replacement"], json!({"id": "minecraft:cave_air"}));
    }

    #[test]
    fn ocean_monuments_become_the_games() {
        let names = [
            "random_prismarine_slab_decoration_processor",
            "random_dark_prismarine_slab_decoration_processor",
            "sand_gravel_processor",
            "random_oxidization_processor",
            "structure_void_processor",
            "air_processor",
            "waterlog_processor",
            "seagrass_processor",
            "random_sponge_processor",
            "leg_processor",
        ];
        let processors: Vec<Value> = names
            .iter()
            .map(|n| json!({"processor_type": format!("betteroceanmonuments:{n}")}))
            .chain([json!({"processor_type": "betterwitchhuts:fence_leg_processor"})])
            .collect();
        let mut list = json!({"processors": processors});
        let mut carried = BTreeMap::new();
        assert!(carry(
            "processor_list",
            &mut list,
            &ProcessorData::default(),
            &mut carried
        ));
        let p = list["processors"].as_array().unwrap();
        let types: Vec<&str> = p
            .iter()
            .map(|p| p["processor_type"].as_str().unwrap())
            .collect();
        let mut want = vec!["horizon:pick", "horizon:pick", "minecraft:rule"];
        want.extend(["horizon:pick"; 4]);
        want.extend([
            "horizon:keep_world_block",
            "horizon:flood_air",
            "horizon:waterlog",
            "minecraft:rule",
            "horizon:pick",
            "horizon:yung_pillar",
            "horizon:yung_pillar",
        ]);
        assert_eq!(types, want);
        assert_eq!(carried.values().sum::<usize>(), 11);
        // Copper stairs keep their properties, each chance on a new float.
        let stairs = &p[5];
        assert_eq!(stairs["block"], "minecraft:oxidized_cut_copper_stairs");
        assert_eq!(
            stairs["picks"][1]["state"]["id"],
            "minecraft:weathered_cut_copper_stairs"
        );
        assert_eq!(
            (stairs["redraw"].clone(), stairs["keep_properties"].clone()),
            (json!(true), json!(true))
        );
        assert_eq!(p[0]["picks"][0]["state"]["properties"]["type"], "top");
        assert_eq!(p[1]["default"]["state"]["id"], "minecraft:water");
        assert_eq!(
            p[12]["target_block_output"]["defaultBlockState"]["id"],
            "minecraft:prismarine_bricks"
        );
        assert_eq!(p[13]["keep_properties"], true);
    }
}
