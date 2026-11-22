use super::*;

fn structure_modifier_base(tag: &str) -> PathBuf {
    let base = biome_base(tag);
    let entries = BTreeMap::from([
        (
            "minecraft:hut",
            r#"{"spawn_overrides":{"monster":{"bounding_box":"full","spawns":[{"type":"minecraft:skeleton","count":1,"weight":10}]}}}"#,
        ),
        ("minecraft:untouched", r#"{ "spawn_overrides": {} }"#),
        ("zinc:nested/tower", r#"{"spawn_overrides":{}}"#),
    ]);
    fs::write(
        base.join("worldgen/structures.json"),
        serde_json::to_vec(&entries).unwrap(),
    )
    .unwrap();
    for (id, text) in entries {
        let (ns, path) = id.split_once(':').unwrap();
        let file = base.join(format!("data/{ns}/worldgen/structure/{path}.json"));
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, text).unwrap();
    }
    base
}

fn structure_modifier_mod(dir: &Path, modifiers: &[(&str, &str)]) -> ContentMod {
    let mut files = vec![("horizon-mod.toml".to_owned(), manifest("zinc", ""))];
    for (name, text) in modifiers {
        files.push((
            format!("data/zinc/neoforge/structure_modifier/{name}.json"),
            text.to_string(),
        ));
    }
    files.push((
        "data/zinc/tags/worldgen/structure/targets.json".into(),
        r#"{"values":["minecraft:hut","zinc:nested/tower"]}"#.into(),
    ));
    folder_mod(
        dir,
        &files
            .iter()
            .map(|(p, t)| (p.as_str(), t.as_str()))
            .collect::<Vec<_>>(),
    )
}

#[test]
fn structure_modifiers_apply_phases_and_reload_without_stale_spawns() {
    use crate::mobs::spawn_selection::StructureSpawns;
    let base = structure_modifier_base("structure-modifiers-reload");
    let scratch = base.parent().unwrap();
    let m = structure_modifier_mod(
        &scratch.join("zinc"),
        &[
            (
                "a_remove",
                r##"{"type":"neoforge:remove_spawns","structures":"#zinc:targets","entity_types":"#minecraft:skeletons"}"##,
            ),
            (
                "b_add",
                r##"{"type":"neoforge:add_spawns","structures":"#zinc:targets","spawners":[{"type":"stray","weight":30,"minCount":1,"maxCount":2},{"type":"zombie","weight":20,"minCount":2,"maxCount":5}]}"##,
            ),
            (
                "c_add",
                r#"{"type":"neoforge:add_spawns","structures":["minecraft:hut","zinc:nested/tower"],"spawners":{"type":"rabbit","weight":12,"minCount":3,"maxCount":3}}"#,
            ),
            (
                "d_add",
                r#"{"type":"neoforge:add_spawns","structures":["minecraft:hut","zinc:nested/tower"],"spawners":{"type":"zombie","weight":7,"minCount":1,"maxCount":1}}"#,
            ),
        ],
    );
    let out = scratch.join("out");
    let warnings = build_fixture(&base, Path::new(""), &[m], &out).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    let loaded = StructureSpawns::load(&out);
    for id in ["minecraft:hut", "zinc:nested/tower"] {
        let monster = loaded.get(id, "monster").unwrap();
        assert_eq!(monster.piece, id == "zinc:nested/tower");
        assert_eq!(
            monster.spawns,
            vec![
                crate::biome::SpawnEntry {
                    entity: "minecraft:zombie".into(),
                    weight: 20,
                    min: 2,
                    max: 5,
                    constant: false
                },
                crate::biome::SpawnEntry {
                    entity: "minecraft:zombie".into(),
                    weight: 7,
                    min: 1,
                    max: 1,
                    constant: true
                }
            ]
        );
        assert_eq!(
            loaded.get(id, "creature").unwrap().spawns,
            vec![crate::biome::SpawnEntry {
                entity: "minecraft:rabbit".into(),
                weight: 12,
                min: 3,
                max: 3,
                constant: true
            }]
        );
    }
    assert!(loaded.get("minecraft:untouched", "monster").is_none());
    let bundle: BTreeMap<String, String> =
        serde_json::from_slice(&fs::read(out.join("worldgen/structures.json")).unwrap()).unwrap();
    let canonical: serde_json::Value = serde_json::from_str(&bundle["zinc:nested/tower"]).unwrap();
    let raw: serde_json::Value = serde_json::from_slice(
        &fs::read(out.join("data/zinc/worldgen/structure/nested/tower.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(canonical, raw);
    assert_eq!(
        fs::read(out.join("data/minecraft/worldgen/structure/untouched.json")).unwrap(),
        fs::read(base.join("data/minecraft/worldgen/structure/untouched.json")).unwrap()
    );
    // Keep the mod enabled while removing its monster modifiers. Rebuilding
    // the same destination must start from vanilla, not the previous overlay.
    let remaining = structure_modifier_mod(
        &scratch.join("remaining"),
        &[(
            "rabbits",
            r#"{"type":"neoforge:add_spawns","structures":"minecraft:hut","spawners":{"type":"rabbit","weight":8,"minCount":1,"maxCount":1}}"#,
        )],
    );
    build_fixture(&base, Path::new(""), &[remaining], &out).unwrap();
    let partial = StructureSpawns::load(&out);
    let monsters = &partial.get("minecraft:hut", "monster").unwrap().spawns;
    assert_eq!(monsters.len(), 1);
    assert_eq!(monsters[0].entity, "minecraft:skeleton");
    assert!(partial.get("zinc:nested/tower", "monster").is_none());
    let rabbits = &partial.get("minecraft:hut", "creature").unwrap().spawns;
    assert_eq!(rabbits.len(), 1);
    assert_eq!(rabbits[0].weight, 8);
    // Finally remove the remaining modifier as well.
    build_fixture(&base, Path::new(""), &[], &out).unwrap();
    let reloaded = StructureSpawns::load(&out);
    assert!(reloaded.get("zinc:nested/tower", "monster").is_none());
    assert!(reloaded.get("minecraft:hut", "creature").is_none());
    assert_eq!(
        reloaded.get("minecraft:hut", "monster").unwrap().spawns[0].entity,
        "minecraft:skeleton"
    );
    fs::remove_dir_all(scratch).unwrap();
}

#[test]
fn structure_modifiers_remove_keeps_empty_override_and_clear_removes_it() {
    use crate::mobs::spawn_selection::StructureSpawns;
    let base = structure_modifier_base("structure-modifiers-clear");
    let scratch = base.parent().unwrap();
    let out = scratch.join("out");
    let m = structure_modifier_mod(
        &scratch.join("remove"),
        &[(
            "remove",
            r#"{"type":"neoforge:remove_spawns","structures":"minecraft:hut","entity_types":"skeleton"}"#,
        )],
    );
    build_fixture(&base, Path::new(""), &[m], &out).unwrap();
    assert!(
        StructureSpawns::load(&out)
            .get("minecraft:hut", "monster")
            .unwrap()
            .spawns
            .is_empty()
    );
    for (name, categories) in [
        ("one", r#", "categories":"monster""#),
        ("list", r#", "categories":["monster","creature"]"#),
        ("all", ""),
    ] {
        let text = format!(
            r#"{{"type":"neoforge:clear_spawns","structures":"minecraft:hut"{categories}}}"#
        );
        let m = structure_modifier_mod(
            &scratch.join(name),
            &[
                ("clear", &text),
                (
                    "rabbit",
                    r#"{"type":"neoforge:add_spawns","structures":"minecraft:hut","spawners":{"type":"rabbit","weight":4,"minCount":1,"maxCount":1}}"#,
                ),
            ],
        );
        build_fixture(&base, Path::new(""), &[m], &out).unwrap();
        assert!(
            StructureSpawns::load(&out)
                .get("minecraft:hut", "monster")
                .is_none()
        );
        assert_eq!(
            StructureSpawns::load(&out)
                .get("minecraft:hut", "creature")
                .is_some(),
            name == "one"
        );
    }
    fs::remove_dir_all(scratch).unwrap();
}

#[test]
fn structure_modifiers_unknown_targets_and_types_warn_and_preserve_base() {
    let base = structure_modifier_base("structure-modifiers-unknown");
    let scratch = base.parent().unwrap();
    let out = scratch.join("out");
    let m = structure_modifier_mod(
        &scratch.join("zinc"),
        &[
            ("none", r#"{"type":"neoforge:none"}"#),
            ("custom", r#"{"type":"zinc:custom"}"#),
            (
                "missing",
                r#"{"type":"neoforge:remove_spawns","structures":"zinc:missing","entity_types":"skeleton"}"#,
            ),
        ],
    );
    let warnings = build_fixture(&base, Path::new(""), &[m], &out).unwrap();
    assert_eq!(warnings.len(), 2, "{warnings:?}");
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("unknown structure zinc:missing"))
    );
    assert_eq!(
        fs::read(out.join("worldgen/structures.json")).unwrap(),
        fs::read(base.join("worldgen/structures.json")).unwrap()
    );
    fs::remove_dir_all(scratch).unwrap();
}

#[test]
fn structure_modifiers_follow_mod_order_and_use_overlaid_definitions() {
    use crate::mobs::spawn_selection::StructureSpawns;
    let base = structure_modifier_base("structure-modifiers-overlay");
    let scratch = base.parent().unwrap();
    let first = folder_mod(
        &scratch.join("first"),
        &[
            ("horizon-mod.toml", &manifest("zinc", "")),
            (
                "data/zinc/worldgen/structure/nested/tower.json",
                r#"{"spawn_overrides":{"monster":{"bounding_box":"piece","spawns":[{"type":"minecraft:skeleton","count":2,"weight":11}]}}}"#,
            ),
            (
                "data/zinc/neoforge/structure_modifier/add.json",
                r#"{"type":"neoforge:add_spawns","structures":"zinc:nested/tower","spawners":{"type":"zombie","weight":5,"minCount":1,"maxCount":1}}"#,
            ),
        ],
    );
    let second = folder_mod(
        &scratch.join("second"),
        &[
            ("horizon-mod.toml", &manifest("post", "")),
            (
                "data/post/neoforge/structure_modifier/add.json",
                r#"{"type":"neoforge:add_spawns","structures":"zinc:nested/tower","spawners":{"type":"stray","weight":9,"minCount":1,"maxCount":1}}"#,
            ),
        ],
    );
    let out = scratch.join("out");
    let warnings = build_fixture(&base, Path::new(""), &[first, second], &out).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    let loaded = StructureSpawns::load(&out);
    let monster = loaded.get("zinc:nested/tower", "monster").unwrap();
    assert!(monster.piece);
    assert_eq!(
        monster
            .spawns
            .iter()
            .map(|s| (s.entity.as_str(), s.weight, s.min))
            .collect::<Vec<_>>(),
        vec![
            ("minecraft:skeleton", 11, 2),
            ("minecraft:zombie", 5, 1),
            ("minecraft:stray", 9, 1)
        ]
    );
    fs::remove_dir_all(scratch).unwrap();
}

#[test]
fn structure_modifiers_reject_malformed_supported_data() {
    for text in [
        r#"{"type":"neoforge:add_spawns","structures":"minecraft:hut","spawners":{"type":"zombie","weight":1,"minCount":4,"maxCount":1}}"#,
        r#"{"type":"neoforge:add_spawns","structures":"minecraft:hut"}"#,
        r#"{"type":"neoforge:clear_spawns","structures":"minecraft:hut","categories":"bogus"}"#,
        r#"{"type":"neoforge:remove_spawns","entity_types":"zombie"}"#,
    ] {
        let error = structures::Modifier::read(
            "zinc",
            "data/zinc/neoforge/structure_modifier/bad.json",
            text,
            &mut Vec::new(),
        )
        .err()
        .expect("malformed modifier must fail");
        assert!(
            error
                .to_string()
                .contains("mod zinc: data/zinc/neoforge/structure_modifier/bad.json"),
            "{error}"
        );
    }
}

#[test]
fn structure_modifiers_and_biome_modifier_reject_out_of_range_spawns() {
    for spawner in [
        r#"{"type":"zombie","weight":1,"minCount":1,"maxCount":2147483648}"#,
        r#"{"type":"zombie","weight":2147483648,"minCount":1,"maxCount":1}"#,
        r#"{"type":"zombie","weight":1,"minCount":4294967297,"maxCount":4294967297}"#,
    ] {
        let structure = format!(
            r#"{{"type":"neoforge:add_spawns","structures":"minecraft:hut","spawners":{spawner}}}"#
        );
        assert!(
            structures::Modifier::read("zinc", "bad.json", &structure, &mut Vec::new()).is_err()
        );
        let biome = format!(
            r#"{{"type":"neoforge:add_spawns","biomes":"minecraft:plains","spawners":{spawner}}}"#
        );
        assert!(biomes::Modifier::read("zinc", "bad.json", &biome, &mut Vec::new()).is_err());
    }
}

#[test]
fn structure_modifiers_boundary_count_samples_through_reader() {
    use crate::mobs::spawn_selection::{StructureSpawns, sample_count};
    use horizon_core::random::LegacyRandom;

    let base = structure_modifier_base("structure-modifiers-boundary");
    let scratch = base.parent().unwrap();
    let m = structure_modifier_mod(
        &scratch.join("zinc"),
        &[(
            "boundary",
            r#"{"type":"neoforge:add_spawns","structures":"minecraft:hut","spawners":{"type":"zombie","weight":2147483647,"minCount":1,"maxCount":2147483647}}"#,
        )],
    );
    let out = scratch.join("out");
    assert!(
        build_fixture(&base, Path::new(""), &[m], &out)
            .unwrap()
            .is_empty()
    );
    let loaded = StructureSpawns::load(&out);
    let entry = loaded
        .get("minecraft:hut", "monster")
        .unwrap()
        .spawns
        .iter()
        .find(|entry| entry.weight == 2147483647)
        .unwrap();
    assert_eq!((entry.min, entry.max), (1, 2147483647));
    let mut random = LegacyRandom::new(1);
    assert_eq!(sample_count(entry, &mut random), 1569548986);
    fs::remove_dir_all(scratch).unwrap();
}

#[test]
fn structure_modifiers_evaluate_conditions_before_required_fields() {
    let base = structure_modifier_base("structure-modifiers-conditions");
    let scratch = base.parent().unwrap();
    let m = structure_modifier_mod(
        &scratch.join("zinc"),
        &[
            (
                "disabled",
                r#"{"neoforge:conditions":[{"type":"neoforge:mod_loaded","modid":"absent"}],"type":"neoforge:add_spawns"}"#,
            ),
            (
                "enabled",
                r#"{"neoforge:conditions":[{"type":"neoforge:mod_loaded","modid":"zinc"}],"type":"neoforge:add_spawns","structures":"minecraft:hut","spawners":{"type":"rabbit","weight":3,"minCount":1,"maxCount":1}}"#,
            ),
        ],
    );
    let out = scratch.join("out");
    assert!(
        build_fixture(&base, Path::new(""), &[m], &out)
            .unwrap()
            .is_empty()
    );
    let loaded = crate::mobs::spawn_selection::StructureSpawns::load(&out);
    assert_eq!(
        loaded.get("minecraft:hut", "creature").unwrap().spawns[0].entity,
        "minecraft:rabbit"
    );
    fs::remove_dir_all(scratch).unwrap();
}

/// Exercise the production resource-root cache and the spawn consumer against
/// prepared vanilla data, including switching back to the unmodified root.
#[test]
#[ignore = "daily"]
fn structure_modifiers_prepared_root_reloads_spawn_overrides() {
    use crate::mobs::spawn_selection::StructureSpawns;
    let Some(base) = prepared() else {
        return;
    };
    let _assets =
        crate::assets::Assets::load(&base).expect("prepared resources must match this build");
    let scratch = std::env::temp_dir().join(format!(
        "horizon-structure-modifier-prepared-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&scratch);
    let m = folder_mod(
        &scratch.join("zinc"),
        &[
            ("horizon-mod.toml", &manifest("zinc", "")),
            (
                "data/zinc/neoforge/structure_modifier/rabbits.json",
                r#"{"type":"neoforge:add_spawns","structures":"minecraft:desert_pyramid","spawners":{"type":"minecraft:rabbit","weight":17,"minCount":2,"maxCount":2}}"#,
            ),
        ],
    );
    let vanilla = StructureSpawns::load(&base);
    assert!(
        vanilla
            .get("minecraft:desert_pyramid", "creature")
            .is_none()
    );
    let cache = scratch.join("cache");
    let root = resource_root_fixture(&base, Path::new(""), &[m], &cache).unwrap();
    let loaded = StructureSpawns::load(&root);
    let rabbits = loaded.get("minecraft:desert_pyramid", "creature").unwrap();
    assert!(rabbits.piece);
    assert_eq!(
        rabbits.spawns,
        vec![crate::biome::SpawnEntry {
            entity: "minecraft:rabbit".into(),
            weight: 17,
            min: 2,
            max: 2,
            constant: true,
        }]
    );
    let restored = resource_root_fixture(&base, Path::new(""), &[], &cache).unwrap();
    assert!(
        StructureSpawns::load(&restored)
            .get("minecraft:desert_pyramid", "creature")
            .is_none()
    );
    fs::remove_dir_all(scratch).unwrap();
}

#[test]
fn structure_modifiers_bundle_spawns_without_modifier_preserve_individual_files() {
    use crate::mobs::spawn_selection::StructureSpawns;
    let base = structure_modifier_base("structure-modifiers-bundle-only");
    let out = base.parent().unwrap().join("out");
    let m = folder_mod(
        &base.parent().unwrap().join("zinc"),
        &[
            ("horizon-mod.toml", &manifest("zinc", "")),
            (
                "data/zinc/worldgen/structure/tower.json",
                r#"{"spawn_overrides":{"monster":{"bounding_box":"piece","spawns":[{"type":"minecraft:zombie","weight":17,"count":{"type":"minecraft:uniform","min_inclusive":2,"max_inclusive":4}}]}}}"#,
            ),
        ],
    );
    assert!(
        build_fixture(&base, Path::new(""), &[m], &out)
            .unwrap()
            .is_empty()
    );
    let loaded = StructureSpawns::load(&out);
    let tower = loaded.get("zinc:tower", "monster").unwrap();
    assert!(tower.piece);
    assert_eq!(
        tower.spawns,
        vec![crate::biome::SpawnEntry {
            entity: "minecraft:zombie".into(),
            weight: 17,
            min: 2,
            max: 4,
            constant: false,
        }]
    );
    assert_eq!(
        loaded.get("minecraft:hut", "monster").unwrap().spawns[0].weight,
        10
    );
    fs::remove_dir_all(base.parent().unwrap()).unwrap();
}
