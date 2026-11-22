use super::*;

#[test]
fn biome_modifier_climate_rejects_malformed_ranges() {
    for climate in [
        r#"{"type":"horizon:climate","temperature":{"min":"warm"}}"#,
        r#"{"type":"horizon:climate","temperature":{"min":null,"max":1}}"#,
        r#"{"type":"horizon:climate","temperature":null}"#,
        r#"{"type":"horizon:climate","temperature":{"min":2,"max":1}}"#,
        r#"{"type":"horizon:climate","temperature":{"minimum":1}}"#,
        r#"{"type":"horizon:climate","temperature":{}}"#,
        r#"{"type":"horizon:climate"}"#,
    ] {
        let base = biome_base("biome-invalid-climate");
        let scratch = base.parent().unwrap().to_owned();
        let modifier = format!(
            r#"{{"type":"neoforge:add_features","biomes":{climate},"features":"zinc:ore_zinc","step":"underground_ores"}}"#
        );
        let m = zinc_mod(&scratch.join("zinc"), &[("bad", &modifier)]);
        assert!(
            build_fixture(&base, Path::new(""), &[m], &scratch.join("out")).is_err(),
            "{climate}"
        );
        fs::remove_dir_all(&scratch).unwrap();
    }
}

#[test]
fn biome_modifier_climate_standalone_includes_bounds() {
    let base = biome_base("biome-climate-alone");
    let scratch = base.parent().unwrap().to_owned();
    let m = zinc_mod(
        &scratch.join("zinc"),
        &[(
            "warm",
            r#"{
        "type":"neoforge:add_features",
        "biomes":{"type":"horizon:climate","temperature":{"min":2,"max":2},"downfall":{"max":0}},
        "features":"zinc:ore_zinc","step":"underground_ores"}"#,
        )],
    );
    let out = scratch.join("out");
    build_fixture(&base, Path::new(""), &[m], &out).unwrap();
    let features = biome_features(&out);
    assert_eq!(
        features["minecraft:desert"][6],
        serde_json::json!(["zinc:ore_zinc"])
    );
    assert_eq!(
        features["minecraft:nether_wastes"][6],
        serde_json::json!(["minecraft:ore_coal", "zinc:ore_zinc"])
    );
    assert_eq!(
        features["minecraft:plains"][6],
        serde_json::json!(["minecraft:ore_coal"])
    );
    fs::remove_dir_all(&scratch).unwrap();
}

#[test]
fn biome_modifier_climate_combines_with_holder_sets() {
    let base = biome_base("biome-climate");
    let scratch = base.parent().unwrap().to_owned();
    let m = zinc_mod(
        &scratch.join("zinc"),
        &[(
            "warm",
            r##"{
        "type":"neoforge:add_features",
        "biomes":{"type":"neoforge:and","values":["#minecraft:is_overworld",
            {"type":"horizon:climate","temperature":{"min":0.8,"max":0.8},
             "downfall":{"min":0.4}}]},
        "features":"zinc:ore_zinc","step":"underground_ores"}"##,
        )],
    );
    let out = scratch.join("out");
    build_fixture(&base, Path::new(""), &[m], &out).unwrap();
    let features = biome_features(&out);
    assert_eq!(
        features["minecraft:plains"][6],
        serde_json::json!(["minecraft:ore_coal", "zinc:ore_zinc"])
    );
    assert_eq!(
        features["minecraft:desert"][0],
        serde_json::json!(["minecraft:fossil"])
    );
    assert_eq!(
        features["minecraft:nether_wastes"][6],
        serde_json::json!(["minecraft:ore_coal"])
    );
    fs::remove_dir_all(&scratch).unwrap();
}

#[test]
fn biome_modifier_climate_nested_or_not() {
    let base = biome_base("biome-climate-or-not");
    let scratch = base.parent().unwrap().to_owned();
    let m = zinc_mod(
        &scratch.join("zinc"),
        &[(
            "select",
            r##"{
        "type":"neoforge:add_features", "features":"zinc:ore_zinc", "step":"underground_ores",
        "biomes":{"type":"neoforge:or","values":["minecraft:plains",
            {"type":"neoforge:not","value":{"type":"horizon:climate","temperature":{"min":1}}}]}}"##,
        )],
    );
    let out = scratch.join("out");
    build_fixture(&base, Path::new(""), &[m], &out).unwrap();
    let features = biome_features(&out);
    assert_eq!(
        features["minecraft:plains"][6],
        serde_json::json!(["minecraft:ore_coal", "zinc:ore_zinc"])
    );
    assert_eq!(
        features["minecraft:nether_wastes"][6],
        serde_json::json!(["minecraft:ore_coal"])
    );
    assert_eq!(
        features["minecraft:desert"][0],
        serde_json::json!(["minecraft:fossil"])
    );
    fs::remove_dir_all(&scratch).unwrap();
}

/// `neoforge:add_features` puts a mod's ore at the end of its step in every
/// biome of an overworld tag, `remove_features` takes a vanilla feature out,
/// and the biomes no modifier names keep their text.
#[test]
fn biome_modifiers_place_mod_features_in_vanilla_biomes() {
    let base = biome_base("biome-modifiers");
    let scratch = base.parent().unwrap().to_owned();
    let m = zinc_mod(
        &scratch.join("zinc"),
        &[
            (
                "add_zinc",
                r##"{"type":"neoforge:add_features","biomes":"#minecraft:is_overworld",
                    "features":"zinc:ore_zinc","step":"underground_ores"}"##,
            ),
            // Adds before removes, whatever the file order.
            (
                "a_no_fossils",
                r#"{"type":"neoforge:remove_features","biomes":["minecraft:desert"],
                    "features":["minecraft:fossil"],"steps":["raw_generation"]}"#,
            ),
            ("nothing", r#"{"type":"neoforge:none"}"#),
        ],
    );
    let out = scratch.join("out");
    build_fixture(&base, Path::new(""), &[m], &out).unwrap();
    let features = biome_features(&out);
    assert_eq!(
        features["minecraft:plains"],
        serde_json::json!([
            [],
            ["minecraft:lake"],
            [],
            [],
            [],
            [],
            ["minecraft:ore_coal", "zinc:ore_zinc"]
        ])
    );
    // A biome with fewer steps gets the ones up to the modifier's.
    assert_eq!(
        features["minecraft:desert"],
        serde_json::json!([[], [], [], [], [], [], ["zinc:ore_zinc"]])
    );
    let text = |root: &Path| -> BTreeMap<String, String> {
        serde_json::from_slice(&fs::read(root.join("worldgen/biomes.json")).unwrap()).unwrap()
    };
    assert_eq!(
        text(&out)["minecraft:nether_wastes"],
        text(&base)["minecraft:nether_wastes"]
    );
    let plains: serde_json::Value = serde_json::from_str(&text(&out)["minecraft:plains"]).unwrap();
    assert_eq!(plains["temperature"], serde_json::json!(0.8));

    // The generator's sorter places vanilla's features where they were and
    // the ore after the coal it follows: the order applyBiomeDecoration uses.
    use horizon_world::worldgen::feature_sort::{FeatureIdentity, build_features_per_step};
    let mut ids: Vec<String> = Vec::new();
    let mut plan = |biomes: &[&str], features: &BTreeMap<String, serde_json::Value>| {
        let lists: Vec<Vec<Vec<FeatureIdentity>>> = biomes
            .iter()
            .map(|b| {
                features[*b]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|step| {
                        step.as_array()
                            .unwrap()
                            .iter()
                            .map(|f| {
                                let f = f.as_str().unwrap().to_owned();
                                let i = ids.iter().position(|x| *x == f).unwrap_or_else(|| {
                                    ids.push(f);
                                    ids.len() - 1
                                });
                                FeatureIdentity(i as u32)
                            })
                            .collect()
                    })
                    .collect()
            })
            .collect();
        build_features_per_step(&lists).unwrap()
    };
    let overworld = ["minecraft:plains", "minecraft:desert"];
    let steps = plan(&overworld, &features);
    let names = |s: usize, ids: &[String]| -> Vec<String> {
        steps[s]
            .features()
            .iter()
            .map(|f| ids[f.0 as usize].clone())
            .collect()
    };
    assert_eq!(names(6, &ids), ["minecraft:ore_coal", "zinc:ore_zinc"]);
    assert_eq!(names(1, &ids), ["minecraft:lake"]);
    let _ = fs::remove_dir_all(&scratch);
}

/// 26.3's feature directory feeds the configured-feature bundle and command
/// registry, including nested paths, while other directories are not loaded.
#[test]
fn configured_features_load_from_the_26_3_path() {
    let base = biome_base("feature-paths");
    let scratch = base.parent().unwrap();
    fs::write(
        base.join("command-registries.json"),
        r#"{"blocks":{},"items":[],"by_registry":{"minecraft:worldgen/feature":{"ids":[]}}}"#,
    )
    .unwrap();
    let feature = r#"{"type":"minecraft:simple_block","config":{}}"#;
    let m = folder_mod(
        &scratch.join("zinc"),
        &[
            ("horizon-mod.toml", &manifest("zinc", "")),
            ("data/zinc/worldgen/feature/nested/new.json", feature),
            ("data/zinc/worldgen/configured_feature/old.json", feature),
        ],
    );
    let root = scratch.join("out");
    build_fixture(&base, Path::new(""), &[m], &root).unwrap();
    let bundle: BTreeMap<String, String> =
        serde_json::from_slice(&fs::read(root.join("worldgen/features.json")).unwrap()).unwrap();
    assert_eq!(
        bundle,
        BTreeMap::from([("zinc:nested/new".into(), feature.into())])
    );
    let commands: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("command-registries.json")).unwrap()).unwrap();
    let ids = commands["by_registry"]["minecraft:worldgen/feature"]["ids"]
        .as_array()
        .unwrap();
    assert_eq!(ids, &[serde_json::json!("zinc:nested/new")]);
    fs::remove_dir_all(scratch).unwrap();
}

/// A world's data pack that replaces a vanilla biome (Terralith's plains)
/// keeps the mods' modifier features, and a pack's own biome its tag puts
/// under the modifier's selector gets them: modifiers apply after
/// every pack. A biome no pack touches stays as the content root has it.
#[test]
fn biome_modifiers_apply_over_world_packs() {
    let base = biome_base("biome-modifiers-packs");
    let scratch = base.parent().unwrap().to_owned();
    let m = zinc_mod(
        &scratch.join("zinc"),
        &[(
            "add_zinc",
            r##"{"type":"neoforge:add_features","biomes":"#minecraft:is_overworld",
                "features":"zinc:ore_zinc","step":"underground_ores"}"##,
        )],
    );
    let out = scratch.join("out");
    build_fixture(&base, &base.join("jars"), &[m], &out).unwrap();
    let mut packs = horizon_world::worldgen::overrides::WorldgenOverrides::default();
    let pack_plains =
        r#"{"temperature":0.5,"features":[[],[],[],[],[],[],["minecraft:ore_coal"]]}"#;
    packs.set("biomes", "minecraft:plains", pack_plains.to_owned());
    packs.set("biomes", "terra:alpine", r#"{"features":[]}"#.to_owned());
    packs.add_tag(
        "biome_tags",
        "minecraft:is_overworld",
        "file/terra",
        r#"{"values":["terra:alpine"]}"#.to_owned(),
    );
    let after = biome_modifiers_over_packs(&out, &packs, &[])
        .unwrap()
        .unwrap_or(packs);
    let features = |id: &str| -> serde_json::Value {
        let text = after.entries("biomes").find(|(i, _)| *i == id).unwrap().1;
        serde_json::from_str::<serde_json::Value>(text).unwrap()["features"].clone()
    };
    assert_eq!(
        features("minecraft:plains"),
        serde_json::json!([
            [],
            [],
            [],
            [],
            [],
            [],
            ["minecraft:ore_coal", "zinc:ore_zinc"]
        ])
    );
    assert_eq!(
        features("terra:alpine"),
        serde_json::json!([[], [], [], [], [], [], ["zinc:ore_zinc"]])
    );
    // The pack's own values stay.
    let plains = after
        .entries("biomes")
        .find(|(i, _)| *i == "minecraft:plains");
    let plains: serde_json::Value = serde_json::from_str(plains.unwrap().1).unwrap();
    assert_eq!(plains["temperature"], serde_json::json!(0.5));
    assert!(
        after
            .entries("biomes")
            .all(|(id, _)| id != "minecraft:desert")
    );
    let _ = fs::remove_dir_all(&scratch);
}

/// A mod with features but no modifier leaves vanilla's biome bundle alone.
#[test]
fn biome_modifiers_none_keep_the_vanilla_biomes() {
    let base = biome_base("biome-modifiers-none");
    let scratch = base.parent().unwrap().to_owned();
    let out = scratch.join("out");
    build_fixture(
        &base,
        Path::new(""),
        &[zinc_mod(&scratch.join("zinc"), &[])],
        &out,
    )
    .unwrap();
    assert_eq!(
        fs::read(out.join("worldgen/biomes.json")).unwrap(),
        fs::read(base.join("worldgen/biomes.json")).unwrap()
    );
    let _ = fs::remove_dir_all(&scratch);
}

/// A mod's own biome joins the biome bundle, its tags and the command registry, and a biome
/// modifier reaches it through its tag; vanilla's biomes stay.
#[test]
fn mod_biomes_join_the_biome_bundle() {
    for with_modifier in [false, true] {
        let base = biome_base(&format!("mod-biomes-{with_modifier}"));
        let scratch = base.parent().unwrap().to_owned();
        fs::write(
            base.join("command-registries.json"),
            r#"{"blocks":{},"items":[],"by_registry":{"minecraft:worldgen/biome":{"ids":[]}}}"#,
        )
        .unwrap();
        let foo = r#"{"temperature":1.5,"downfall":0.2,"features":[[],[],[],[],[],[],[]]}"#;
        let mut files = vec![
            ("horizon-mod.toml", manifest("zinc", "")),
            (
                "data/zinc/worldgen/feature/ore_zinc.json",
                r#"{"type":"minecraft:ore","config":{}}"#.to_owned(),
            ),
            (
                "data/zinc/worldgen/placed_feature/ore_zinc.json",
                r#"{"feature":"zinc:ore_zinc","placement":[]}"#.to_owned(),
            ),
            ("data/zinc/worldgen/biome/foo.json", foo.to_owned()),
            (
                "data/zinc/tags/worldgen/biome/hot.json",
                r#"{"values":["zinc:foo"]}"#.to_owned(),
            ),
        ];
        if with_modifier {
            files.push((
                "data/zinc/neoforge/biome_modifier/ores.json",
                r##"{"type":"neoforge:add_features","biomes":"#zinc:hot","features":"zinc:ore_zinc","step":"underground_ores"}"##.to_owned(),
            ));
        }
        let files: Vec<(&str, &str)> = files.iter().map(|(a, b)| (*a, b.as_str())).collect();
        let m = folder_mod(&scratch.join("zinc"), &files);
        let out = scratch.join("out");
        build_fixture(&base, Path::new(""), &[m], &out).unwrap();

        let bundle: BTreeMap<String, String> =
            serde_json::from_slice(&fs::read(out.join("worldgen/biomes.json")).unwrap()).unwrap();
        let vanilla: BTreeMap<String, String> =
            serde_json::from_slice(&fs::read(base.join("worldgen/biomes.json")).unwrap()).unwrap();
        assert_eq!(bundle["minecraft:plains"], vanilla["minecraft:plains"]);
        let foo: serde_json::Value = serde_json::from_str(&bundle["zinc:foo"]).unwrap();
        let ores = match with_modifier {
            true => serde_json::json!(["zinc:ore_zinc"]),
            false => serde_json::json!([]),
        };
        assert_eq!(foo["features"][6], ores, "modifier: {with_modifier}");
        let tags: BTreeMap<String, String> =
            serde_json::from_slice(&fs::read(out.join("worldgen/biome_tags.json")).unwrap())
                .unwrap();
        let hot: serde_json::Value = serde_json::from_str(&tags["zinc:hot"]).unwrap();
        assert_eq!(hot["values"], serde_json::json!(["zinc:foo"]));
        let commands: serde_json::Value =
            serde_json::from_slice(&fs::read(out.join("command-registries.json")).unwrap())
                .unwrap();
        assert_eq!(
            commands["by_registry"]["minecraft:worldgen/biome"]["ids"],
            serde_json::json!(["zinc:foo"])
        );
        let mods: Vec<String> = crate::biome::mod_biome_files(&out)
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(mods, ["zinc:foo"]);
        let _ = fs::remove_dir_all(&scratch);
    }
}

/// An unknown biome, biome tag, feature, entity type, step or modifier type is
/// skipped with a warning or fails the build with the mod and file named.
#[test]
fn biome_modifiers_refuse_unknown_names() {
    let base = biome_base("biome-modifiers-errors");
    let scratch = base.parent().unwrap().to_owned();
    // A name the root doesn't have, or a type not read yet: that modifier is
    // skipped with a warning (another mod's ids, `#c:` tags and its own serializers
    // are common in real mods), and the mods still load.
    for (i, (modifier, warning)) in [
        (
            r##"{"type":"neoforge:add_features","biomes":"#minecraft:is_moon",
                "features":"zinc:ore_zinc","step":"underground_ores"}"##,
            "unknown biome tag #minecraft:is_moon",
        ),
        (
            r#"{"type":"neoforge:add_features","biomes":"minecraft:moon",
                "features":"zinc:ore_zinc","step":"underground_ores"}"#,
            "unknown biome minecraft:moon",
        ),
        (
            r#"{"type":"neoforge:add_features","biomes":"minecraft:plains",
                "features":"zinc:ore_tin","step":"underground_ores"}"#,
            "unknown placed feature zinc:ore_tin",
        ),
        (
            r#"{"type":"neoforge:add_spawns","biomes":"minecraft:plains",
                "spawners":{"type":"zinc:goblin","weight":10,"minCount":1,"maxCount":2}}"#,
            "unknown entity type zinc:goblin",
        ),
        (
            r##"{"type":"neoforge:remove_spawns","biomes":"minecraft:plains",
                "entity_types":"#minecraft:is_goblin"}"##,
            "unknown entity type tag #minecraft:is_goblin",
        ),
        (
            r#"{"type":"examplemod:custom","biomes":"minecraft:plains"}"#,
            "biome modifier type \"examplemod:custom\" skipped",
        ),
        (
            r#"{"type":"neoforge:add_features","biomes":"minecraft:plains",
                "features":{"type":"horizon:climate","temperature":{"min":1}},"step":"underground_ores"}"#,
            "placed feature set type \"horizon:climate\" is not read",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let m = zinc_mod(&scratch.join(format!("zinc{i}")), &[("bad", modifier)]);
        let warnings = build_fixture(&base, Path::new(""), &[m], &scratch.join(format!("out{i}"))).unwrap();
        assert!(
            warnings.iter().any(|w| w.contains(warning)
                && w.contains("mod zinc: data/zinc/neoforge/biome_modifier/bad.json")),
            "{warnings:?}"
        );
    }
    // A malformed modifier of a type Horizon reads is the mod's error.
    for (i, (modifier, err)) in [
        (
            r#"{"type":"neoforge:add_features","biomes":"minecraft:plains",
                "features":"zinc:ore_zinc","step":"ores"}"#,
            "unknown generation step \"ores\"",
        ),
        (
            r#"{"type":"neoforge:add_spawns","biomes":"minecraft:plains",
                "spawners":{"type":"minecraft:zombie","weight":10,"minCount":5,"maxCount":2}}"#,
            "minCount (5) > maxCount (2)",
        ),
        (
            r#"{"type":"neoforge:add_spawn_costs","biomes":"minecraft:plains",
                "entity_types":"minecraft:zombie","spawn_cost":{"energy_budget":0.7}}"#,
            "spawn_cost: no charge",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let m = zinc_mod(&scratch.join(format!("zinc-err{i}")), &[("bad", modifier)]);
        let e = build_fixture(
            &base,
            Path::new(""),
            &[m],
            &scratch.join(format!("out-err{i}")),
        )
        .unwrap_err()
        .to_string();
        assert!(e.contains(err), "{e}");
        assert!(
            e.contains("mod zinc: data/zinc/neoforge/biome_modifier/bad.json"),
            "{e}"
        );
    }
    let _ = fs::remove_dir_all(&scratch);
}

/// `add_spawns`, `remove_spawns`, `add_spawn_costs` and `remove_spawn_costs`
/// rewrite `minecraft:gameplay/natural_mob_spawns` in the biomes they name:
/// additions before removals regardless of file order, spawners placed under
/// their entity type's category with `minCount`/`maxCount` converted to
/// `count`, and untouched biomes left byte for byte as they were.
#[test]
fn biome_modifiers_change_mob_spawns_and_spawn_costs() {
    let base = biome_base("biome-modifiers-spawns");
    let scratch = base.parent().unwrap().to_owned();
    let m = zinc_mod(
        &scratch.join("zinc"),
        &[
            // Listed before `add_spawns`, but removals run after additions:
            // takes out vanilla's `minecraft:skeleton` and the added
            // `minecraft:stray` via `#minecraft:skeletons`.
            (
                "a_no_skeletons",
                r##"{"type":"neoforge:remove_spawns","biomes":"#minecraft:is_overworld",
                    "entity_types":"#minecraft:skeletons"}"##,
            ),
            (
                "a_no_skeleton_cost",
                r##"{"type":"neoforge:remove_spawn_costs","biomes":"minecraft:plains",
                    "entity_types":"#minecraft:skeletons"}"##,
            ),
            // Single spawner object (`minCount == maxCount` -> integer `count`)
            // on a biome that had no `attributes` at all (`desert`), and on `plains`.
            (
                "add_rabbits",
                r##"{"type":"neoforge:add_spawns","biomes":"#minecraft:is_overworld",
                    "spawners":{"type":"minecraft:rabbit","weight":12,"minCount":3,"maxCount":3}}"##,
            ),
            // Array of spawners (`minCount < maxCount` -> `minecraft:uniform`).
            (
                "add_monsters",
                r#"{"type":"forge:add_spawns","biomes":"plains",
                    "spawners":[
                        {"type":"stray","weight":80,"minCount":1,"maxCount":4},
                        {"type":"minecraft:zombie","weight":20,"minCount":2,"maxCount":5}
                    ]}"#,
            ),
            (
                "add_zombie_cost",
                r#"{"type":"neoforge:add_spawn_costs","biomes":["minecraft:plains","minecraft:desert"],
                    "entity_types":["minecraft:zombie","minecraft:stray"],
                    "spawn_cost":{"energy_budget":0.85,"charge":0.2}}"#,
            ),
        ],
    );
    let out = scratch.join("out");
    let warnings = build_fixture(&base, Path::new(""), &[m], &out).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");

    let text = |root: &Path| -> BTreeMap<String, String> {
        serde_json::from_slice(&fs::read(root.join("worldgen/biomes.json")).unwrap()).unwrap()
    };
    let out_biomes = text(&out);
    // Untouched biome keeps its exact bytes.
    assert_eq!(
        out_biomes["minecraft:nether_wastes"],
        text(&base)["minecraft:nether_wastes"]
    );

    let plains: serde_json::Value = serde_json::from_str(&out_biomes["minecraft:plains"]).unwrap();
    let plains_spawns = &plains["attributes"]["minecraft:gameplay/natural_mob_spawns"]["argument"];
    // `minecraft:skeleton` and the added `minecraft:stray` were removed by
    // `remove_spawns`, while vanilla's zombie, the second zombie spawner, and
    // the rabbit in `creature` remain.
    assert_eq!(
        plains_spawns["spawns_by_category"],
        serde_json::json!({
            "monster": [
                {"type": "minecraft:zombie", "count": 4, "weight": 95},
                {
                    "type": "minecraft:zombie",
                    "count": {"type": "minecraft:uniform", "min_inclusive": 2, "max_inclusive": 5},
                    "weight": 20
                }
            ],
            "creature": [
                {"type": "minecraft:rabbit", "count": 3, "weight": 12}
            ]
        })
    );
    // `skeleton` and `stray` costs were removed on `plains`; `zombie` cost remains.
    assert_eq!(
        plains_spawns["spawn_costs"],
        serde_json::json!({
            "minecraft:zombie": {"energy_budget": 0.85, "charge": 0.2}
        })
    );

    let desert: serde_json::Value = serde_json::from_str(&out_biomes["minecraft:desert"]).unwrap();
    let desert_spawns = &desert["attributes"]["minecraft:gameplay/natural_mob_spawns"]["argument"];
    assert_eq!(
        desert_spawns["spawns_by_category"],
        serde_json::json!({
            "creature": [
                {"type": "minecraft:rabbit", "count": 3, "weight": 12}
            ]
        })
    );
    assert_eq!(
        desert_spawns["spawn_costs"],
        serde_json::json!({
            "minecraft:zombie": {"energy_budget": 0.85, "charge": 0.2},
            "minecraft:stray": {"energy_budget": 0.85, "charge": 0.2}
        })
    );
    let _ = fs::remove_dir_all(&scratch);
}

/// Over the real prepared resources: a mod's ore, added to
/// `#minecraft:is_overworld` by a biome modifier, generates in a fresh
/// world's chunk; without the modifier the same placed feature does not
/// (its `biome` filter only passes in biomes that list it).
#[test]
#[ignore = "daily"]
fn content_root_generates_a_biome_modifier_ore() {
    use crate::streaming::{ChunkGenerator, Proto};
    let Some(base) = prepared() else {
        return;
    };
    let scratch = std::env::temp_dir().join(format!("horizon-mod-ore-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let ore = |modifier: bool| -> usize {
        let mut files = vec![
            ("horizon-mod.toml", manifest("zinc", "")),
            (
                "data/zinc/horizon/block/zinc_ore.json",
                r#"{"template": "minecraft:iron_ore"}"#.to_owned(),
            ),
            (
                "data/zinc/worldgen/feature/ore_zinc.json",
                r##"{"type": "minecraft:ore", "size": 9, "discard_chance_on_air_exposure": 0.0,
                    "targets": [{"target": {"predicate_type": "minecraft:tag_match",
                        "tag": "minecraft:stone_ore_replaceables"},
                        "state": "zinc:zinc_ore"}]}"##
                    .to_owned(),
            ),
            (
                "data/zinc/worldgen/placed_feature/ore_zinc.json",
                r#"{"feature": "zinc:ore_zinc", "placement": [
                    {"type": "minecraft:count", "count": 40}, {"type": "minecraft:in_square"},
                    {"type": "minecraft:height_range", "height": {"type": "minecraft:uniform",
                        "min_inclusive": {"absolute": -32}, "max_inclusive": {"absolute": 48}}},
                    {"type": "minecraft:biome"}]}"#
                    .to_owned(),
            ),
        ];
        if modifier {
            files.push((
                "data/zinc/neoforge/biome_modifier/add_zinc.json",
                r##"{"type": "neoforge:add_features", "biomes": "#minecraft:is_overworld",
                    "features": "zinc:ore_zinc", "step": "underground_ores"}"##
                    .to_owned(),
            ));
        }
        let files: Vec<(&str, &str)> = files.iter().map(|(a, b)| (*a, b.as_str())).collect();
        let dir = scratch.join(format!("zinc-{modifier}"));
        let m = folder_mod(&dir, &files);
        let root = resource_root_fixture(&base, Path::new(""), &[m], &dir.join("cache")).unwrap();
        let assets = Assets::load(&root).unwrap();
        let zinc = assets.registry.default_state("zinc:zinc_ore").unwrap().0;
        let biomes = crate::biome::Biomes::load(
            &horizon_assets::resource_source::ResourceRoot::directory(root.to_path_buf()),
        )
        .unwrap();
        let item_data =
            crate::test_resources::item_data(&root, DataLoadOrigin::ServerDataLoad).unwrap();
        let terrain_resources = crate::vanillagen::TerrainResources {
            data: None,
            dir: &root.join("worldgen"),
            registry: assets.registry,
            biomes: &biomes,
            item_data: &item_data,
        };
        let terrain = crate::vanillagen::VanillaTerrain::new(terrain_resources, 1234).unwrap();
        let mut worker = terrain.worker();
        let mut region: [Proto; 9] =
            std::array::from_fn(|i| terrain.proto(&mut worker, i as i32 % 3 - 1, i as i32 / 3 - 1));
        terrain.decorate(&mut worker, &mut region);
        let [_, _, _, _, center, ..] = region;
        let chunk = terrain.finish(center);
        let mut count = 0;
        for y in chunk.min_y()..chunk.max_y() {
            for z in 0..16 {
                for x in 0..16 {
                    count += usize::from(chunk.get(x, y, z) == zinc);
                }
            }
        }
        count
    };
    assert!(ore(true) > 0, "the modifier's ore generates");
    assert_eq!(ore(false), 0, "no modifier, no ore");
    let _ = fs::remove_dir_all(&scratch);
}

/// `biomes` may be one id or a list of ids as well as a tag (NeoForge's
/// HolderSet forms); an id may leave out `minecraft:`.
#[test]
fn biome_modifiers_take_one_biome_or_a_list() {
    let base = biome_base("biome-modifiers-lists");
    let scratch = base.parent().unwrap().to_owned();
    let m = zinc_mod(
        &scratch.join("zinc"),
        &[
            (
                "list",
                r#"{"type":"neoforge:add_features","biomes":["minecraft:plains","nether_wastes"],
                    "features":"zinc:ore_zinc","step":"underground_ores"}"#,
            ),
            (
                "one",
                r#"{"type":"neoforge:add_features","biomes":"minecraft:desert",
                    "features":["minecraft:lake"],"step":"lakes"}"#,
            ),
        ],
    );
    let out = scratch.join("out");
    build_fixture(&base, Path::new(""), &[m], &out).unwrap();
    let features = biome_features(&out);
    assert_eq!(
        features["minecraft:plains"][6],
        serde_json::json!(["minecraft:ore_coal", "zinc:ore_zinc"])
    );
    assert_eq!(
        features["minecraft:nether_wastes"][6],
        serde_json::json!(["minecraft:ore_coal", "zinc:ore_zinc"])
    );
    assert_eq!(
        features["minecraft:desert"],
        serde_json::json!([["minecraft:fossil"], ["minecraft:lake"]])
    );
    let _ = fs::remove_dir_all(&scratch);
}
