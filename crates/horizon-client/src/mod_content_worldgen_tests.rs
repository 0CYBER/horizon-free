use super::*;

/// A mod's own carver joins the carvers bundle, so a biome of the mod that
/// names it finds it. One of a type the world can't decode, or whose load
/// conditions fail, is left out and listed for the worldgen loader.
#[test]
#[ignore = "daily"]
fn content_root_carries_a_mods_carver() {
    let Some(base) = prepared() else {
        return;
    };
    let scratch = std::env::temp_dir().join(format!("horizon-mod-carver-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let dir = scratch.join("hollows");
    let vanilla: BTreeMap<String, String> =
        serde_json::from_slice(&fs::read(base.join("worldgen/carvers.json")).unwrap()).unwrap();
    let biomes: BTreeMap<String, String> =
        serde_json::from_slice(&fs::read(base.join("worldgen/biomes.json")).unwrap()).unwrap();
    let mut hollow: serde_json::Value = serde_json::from_str(&biomes["minecraft:plains"]).unwrap();
    let named = serde_json::json!(["hollows:deep_cave", "hollows:vent", "hollows:gated"]);
    hollow["carvers"] = named.clone();
    let hollow = hollow.to_string();
    let mut gated: serde_json::Value = serde_json::from_str(&vanilla["minecraft:cave"]).unwrap();
    gated["neoforge:conditions"] =
        serde_json::json!([{"type": "neoforge:mod_loaded", "modid": "absent"}]);
    let gated = gated.to_string();
    let files = [
        (
            "horizon-mod.toml",
            "id = \"hollows\"\nname = \"Hollows\"\nversion = \"1.0\"\napi = \"0.2\"\nside = \"both\"\ncapabilities = [\"registries\"]\n",
        ),
        // Shaped as 26.3's own cave carver, under the mod's id.
        (
            "data/hollows/worldgen/carver/deep_cave.json",
            vanilla["minecraft:cave"].as_str(),
        ),
        // A carver type of the mod's Java.
        (
            "data/hollows/worldgen/carver/vent.json",
            r#"{"type": "hollows:vent", "config": {"probability": 0.1}}"#,
        ),
        // A cave carver only for when a mod nobody loads is there.
        ("data/hollows/worldgen/carver/gated.json", gated.as_str()),
        ("data/hollows/worldgen/biome/hollow.json", hollow.as_str()),
    ];
    for (name, text) in files {
        let path = dir.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    let package = horizon_mod_package::Package::read(&dir).unwrap();
    let m = ContentMod {
        id: "hollows".into(),
        content: Content::read(&dir, &package.manifest).unwrap(),
        jars: Vec::new(),
    };
    let root = resource_root_fixture(
        &base,
        Path::new(""),
        std::slice::from_ref(&m),
        &scratch.join("cache"),
    )
    .unwrap();
    let carvers: BTreeMap<String, String> =
        serde_json::from_slice(&fs::read(root.join("worldgen/carvers.json")).unwrap()).unwrap();
    assert_eq!(carvers["hollows:deep_cave"], vanilla["minecraft:cave"]);
    assert!(carvers.contains_key("minecraft:cave"));
    assert!(!carvers.contains_key("hollows:vent"));
    assert!(!carvers.contains_key("hollows:gated"));
    // The biome keeps every name: the worldgen loader drops the left-out ones
    // after a world's data packs are in, unless one supplies them.
    let biomes: BTreeMap<String, String> =
        serde_json::from_slice(&fs::read(root.join("worldgen/biomes.json")).unwrap()).unwrap();
    let hollow: serde_json::Value = serde_json::from_str(&biomes["hollows:hollow"]).unwrap();
    assert_eq!(hollow["carvers"], named);
    let left_out: Vec<String> =
        serde_json::from_slice(&fs::read(root.join("worldgen/carvers_left_out.json")).unwrap())
            .unwrap();
    assert_eq!(left_out, ["hollows:gated", "hollows:vent"]);
    let _ = fs::remove_dir_all(&scratch);
}

/// A structure mod's worldgen reaches the worldgen loader: its structure,
/// set, pool and templates join the bundles, and its biome tag keeps only
/// what the loader knows.
#[test]
#[ignore = "daily"]
fn content_root_carries_a_structure_mod() {
    let Some(base) = prepared() else {
        return;
    };
    let scratch = std::env::temp_dir().join(format!("horizon-mod-worldgen-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let dir = scratch.join("huts");
    let files = [
        (
            "horizon-mod.toml",
            "id = \"huts\"\nname = \"Huts\"\nversion = \"1.0\"\napi = \"0.2\"\nside = \"both\"\ncapabilities = [\"registries\"]\n",
        ),
        (
            "data/huts/worldgen/structure/hut.json",
            r##"{"type": "minecraft:jigsaw", "biomes": "#huts:has_hut", "step": "surface_structures",
                "spawn_overrides": {}, "terrain_adaptation": "beard_thin", "start_pool": "huts:hut/start",
                "size": 1, "start_height": {"absolute": 0}, "project_start_to_heightmap": "WORLD_SURFACE_WG",
                "max_distance_from_center": 80}"##,
        ),
        (
            "data/huts/worldgen/template_pool/hut/start.json",
            r#"{"fallback": "minecraft:empty", "elements": [{"weight": 1, "element": {
                "element_type": "minecraft:single_pool_element", "location": "huts:hut",
                "projection": "rigid", "processors": "minecraft:empty"}}]}"#,
        ),
        (
            "data/huts/worldgen/structure_set/huts.json",
            r#"{"structures": [{"structure": "huts:hut", "weight": 1}],
                "placement": {"type": "minecraft:random_spread", "salt": 7, "spacing": 4, "separation": 2}}"#,
        ),
        (
            "data/huts/tags/worldgen/biome/has_hut.json",
            r##"{"values": ["#minecraft:is_overworld", "#c:is_plains", "othermod:glade", "minecraft:plains"]}"##,
        ),
        // Additions to vanilla's set, pool and processor list; a target nobody
        // defines is skipped. A lower priority goes first.
        (
            "data/huts/worldgen/structure_set/added/villages.json",
            r#"{"horizon:add_to": "minecraft:villages",
                "structures": [{"structure": "huts:hut", "weight": 3}]}"#,
        ),
        (
            "data/huts/worldgen/template_pool/added/houses.json",
            r#"{"horizon:add_to": ["othermod:houses", "minecraft:village/plains/houses"],
                "elements": [{"weight": 2, "element": {"element_type": "minecraft:single_pool_element",
                "location": "huts:hut", "projection": "rigid", "processors": "minecraft:empty"}}]}"#,
        ),
        (
            "data/huts/worldgen/template_pool/added/more_houses.json",
            r#"{"horizon:add_to": "minecraft:village/plains/houses", "horizon:priority": 10,
                "elements": [{"weight": 1, "element": {"element_type": "minecraft:empty_pool_element"}}]}"#,
        ),
        (
            "data/huts/worldgen/processor_list/added/mossy.json",
            r#"{"horizon:add_to": "minecraft:mossify_10_percent",
                "processors": [{"processor_type": "minecraft:nop"}]}"#,
        ),
    ];
    for (name, text) in files {
        let path = dir.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    // The vanilla house's bytes belong to the mod; prepared file permissions do not.
    let houses = base.join("worldgen/structure/minecraft/village/plains/houses");
    let template = fs::read(houses.join("plains_small_house_1.nbt")).unwrap();
    fs::create_dir_all(dir.join("data/huts/structure")).unwrap();
    fs::write(dir.join("data/huts/structure/hut.nbt"), template).unwrap();
    let package = horizon_mod_package::Package::read(&dir).unwrap();
    let m = ContentMod {
        id: "huts".into(),
        content: Content::read(&dir, &package.manifest).unwrap(),
        jars: Vec::new(),
    };
    let root = resource_root_fixture(
        &base,
        Path::new(""),
        std::slice::from_ref(&m),
        &scratch.join("cache"),
    )
    .unwrap();
    let resources = resolved_resources(
        &base,
        root.clone(),
        Path::new(""),
        std::slice::from_ref(&m),
        scratch.join("cache"),
    )
    .unwrap();
    assert!(
        resources
            .data_changes()
            .contains("data/huts/structure/hut.nbt")
    );
    assert!(!root.join("worldgen/structure/huts/hut.nbt").exists());
    let tags: BTreeMap<String, String> =
        serde_json::from_slice(&fs::read(root.join("worldgen/biome_tags.json")).unwrap()).unwrap();
    let hut: serde_json::Value = serde_json::from_str(&tags["huts:has_hut"]).unwrap();
    assert_eq!(
        hut["values"],
        serde_json::json!(["#minecraft:is_overworld", "minecraft:plains"])
    );
    // The additions join vanilla's entries at the end and are no set or pool
    // of their own.
    let bundle = |name: &str, base: &Path| -> BTreeMap<String, String> {
        serde_json::from_slice(&fs::read(base.join("worldgen").join(name)).unwrap()).unwrap()
    };
    for (name, id, list, added) in [
        (
            "structure_sets.json",
            "minecraft:villages",
            "structures",
            vec![serde_json::json!({"structure": "huts:hut", "weight": 3})],
        ),
        (
            "template_pools.json",
            "minecraft:village/plains/houses",
            "elements",
            vec![
                serde_json::json!({"weight": 1, "element": {"element_type": "minecraft:empty_pool_element"}}),
                serde_json::json!({"weight": 2, "element": {"element_type": "minecraft:single_pool_element",
                "location": "huts:hut", "projection": "rigid", "processors": "minecraft:empty"}}),
            ],
        ),
        (
            "processor_lists.json",
            "minecraft:mossify_10_percent",
            "processors",
            vec![serde_json::json!({"processor_type": "minecraft:nop"})],
        ),
    ] {
        let (ours, vanilla) = (bundle(name, &root), bundle(name, &base));
        let entries = |text: &str| -> Vec<serde_json::Value> {
            let v: serde_json::Value = serde_json::from_str(text).unwrap();
            v[list].as_array().unwrap().clone()
        };
        let mut expected = entries(&vanilla[id]);
        expected.extend(added);
        assert_eq!(entries(&ours[id]), expected, "{id}");
        assert!(!ours.keys().any(|k| k.starts_with("huts:added/")), "{name}");
        assert!(!ours.contains_key("othermod:houses"));
    }

    // The commands suggest the mod's structure, set and pool.
    let registries: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("command-registries.json")).unwrap()).unwrap();
    for (registry, id) in [
        ("minecraft:worldgen/structure", "huts:hut"),
        ("minecraft:worldgen/structure_set", "huts:huts"),
        ("minecraft:worldgen/template_pool", "huts:hut/start"),
    ] {
        let ids = registries["by_registry"][registry]["ids"]
            .as_array()
            .unwrap();
        assert!(ids.iter().any(|v| v == id), "{registry} lacks {id}");
        assert!(
            ids.iter()
                .any(|v| v.as_str().unwrap().starts_with("minecraft:"))
        );
    }

    let assets = Assets::load(&root).unwrap();
    let biomes = crate::biome::Biomes::load(
        &horizon_assets::resource_source::ResourceRoot::directory(root.to_path_buf()),
    )
    .unwrap();
    let item_data =
        crate::test_resources::item_data(&root, DataLoadOrigin::ServerDataLoad).unwrap();
    let terrain_resources = crate::vanillagen::TerrainResources {
        data: Some(resources.data_changes()),
        dir: &root.join("worldgen"),
        registry: assets.registry,
        biomes: &biomes,
        item_data: &item_data,
    };
    let terrain = crate::vanillagen::VanillaTerrain::new(terrain_resources, 1234).unwrap();
    let (name, at) = terrain
        .locate(&["huts:hut".to_owned()], [0, 64, 0], 16)
        .expect("the mod's structure generates");
    assert_eq!(name, "huts:hut");
    assert!(at[0].abs() < 1024 && at[2].abs() < 1024, "{at:?}");
    let _ = fs::remove_dir_all(&scratch);
}

/// `FlatLevelSource.createState`: a superflat world runs the structure sets its
/// `structure_overrides` allow (the classic flat world: strongholds and villages) on its
/// flat ground, and no other set, with the layers where the terrain would be.
#[test]
#[ignore = "daily"]
fn a_superflat_world_runs_its_structure_sets_on_flat_ground() {
    use crate::streaming::ChunkGenerator;
    let Some(base) = prepared() else {
        return;
    };
    let assets = Assets::load(&base).unwrap();
    let biomes = crate::biome::Biomes::load(
        &horizon_assets::resource_source::ResourceRoot::directory(base.to_path_buf()),
    )
    .unwrap();
    let item_data =
        crate::test_resources::item_data(&base, DataLoadOrigin::ServerDataLoad).unwrap();
    let terrain_resources = crate::vanillagen::TerrainResources {
        data: None,
        dir: &base.join("worldgen"),
        registry: assets.registry,
        biomes: &biomes,
        item_data: &item_data,
    };
    let flat = horizon_world::worldgen::generator::FlatSettings {
        layers: vec![
            ("minecraft:bedrock".into(), 1),
            ("minecraft:dirt".into(), 2),
            ("minecraft:grass_block".into(), 1),
        ],
        biome: "minecraft:plains".into(),
        structure_overrides: Some(vec![
            "minecraft:strongholds".into(),
            "minecraft:villages".into(),
        ]),
        features: false,
        lakes: false,
    };
    let terrain = crate::vanillagen::VanillaTerrain::for_flat(
        terrain_resources,
        &Default::default(),
        1234,
        &flat,
        true,
    )
    .unwrap();
    // A village and a stronghold are found; a mineshaft is not in the overrides.
    let village = terrain.locate(&["minecraft:village_plains".to_owned()], [0, 64, 0], 12);
    assert!(village.is_some(), "no plains village near the origin");
    assert!(
        terrain
            .locate(&["minecraft:stronghold".to_owned()], [0, 64, 0], 12)
            .is_some()
    );
    assert!(
        terrain
            .locate(&["minecraft:mineshaft".to_owned()], [0, 64, 0], 12)
            .is_none()
    );
    // The layers: bedrock, two dirt and grass from the level's minimum Y, air above.
    let mut worker = terrain.worker();
    let chunk = terrain.finish(terrain.proto(&mut worker, 3, 3));
    let name = |y| {
        let id = chunk.block(3 * 16 + 5, y, 3 * 16 + 7);
        assets.registry.state(id).unwrap().name.clone()
    };
    assert_eq!(
        [-65, -64, -63, -62, -61, -60].map(name),
        [
            "minecraft:air",
            "minecraft:bedrock",
            "minecraft:dirt",
            "minecraft:dirt",
            "minecraft:grass_block",
            "minecraft:air"
        ]
        .map(String::from)
    );
}

/// `adjustGenerationSettings`: the Overworld preset decorates its plains with grass,
/// flowers and trees even with no structure set (`structure_overrides: []`, or
/// `generate_structures` off), and without `features` the same layers stay bare.
#[test]
#[ignore = "daily"]
fn a_decorated_superflat_world_without_structures_grows_its_features() {
    use crate::streaming::ChunkGenerator;
    let Some(base) = prepared() else {
        return;
    };
    let assets = Assets::load(&base).unwrap();
    let biomes = crate::biome::Biomes::load(
        &horizon_assets::resource_source::ResourceRoot::directory(base.to_path_buf()),
    )
    .unwrap();
    let item_data = crate::test_resources::item_data(
        &base,
        horizon_world::native_load_context::DataLoadOrigin::ServerDataLoad,
    )
    .unwrap();
    // Blocks above the grass (the top layer is at the level's minimum Y + 63) in the
    // chunk at 4, 4 once its 3×3 region is decorated.
    let above_ground = |features| {
        let flat = horizon_world::worldgen::generator::FlatSettings {
            layers: vec![
                ("minecraft:bedrock".into(), 1),
                ("minecraft:stone".into(), 59),
                ("minecraft:dirt".into(), 3),
                ("minecraft:grass_block".into(), 1),
            ],
            biome: "minecraft:plains".into(),
            structure_overrides: Some(vec![]),
            features,
            lakes: false,
        };
        let terrain = crate::vanillagen::VanillaTerrain::for_flat(
            crate::vanillagen::TerrainResources {
                data: None,
                dir: &base.join("worldgen"),
                registry: assets.registry,
                biomes: &biomes,
                item_data: &item_data,
            },
            &Default::default(),
            1234,
            &flat,
            false,
        )
        .unwrap();
        let mut worker = terrain.worker();
        let mut region: [_; 9] =
            std::array::from_fn(|i| terrain.proto(&mut worker, 3 + i as i32 % 3, 3 + i as i32 / 3));
        terrain.decorate(&mut worker, &mut region);
        let [_, _, _, _, center, ..] = region;
        let chunk = terrain.finish(center);
        let mut found = Vec::new();
        for y in 0..16 {
            for z in 64..80 {
                for x in 64..80 {
                    let name = &assets.registry.state(chunk.block(x, y, z)).unwrap().name;
                    if name != "minecraft:air" {
                        found.push(name.clone());
                    }
                }
            }
        }
        found
    };
    let decorated = above_ground(true);
    assert!(
        decorated.iter().any(|n| n == "minecraft:short_grass"),
        "{decorated:?}"
    );
    assert_eq!(above_ground(false), Vec::<String>::new());
}

/// `createForFlat` searches the stronghold rings with seed 0, so a superflat world's
/// strongholds are in the same place on every seed; a layer that is not opaque to
/// `MOTION_BLOCKING` is `null`, left out of `buildTerrain` for the fill after the structures.
#[test]
#[ignore = "daily"]
fn a_superflat_world_keeps_its_strongholds_on_every_seed_and_leaves_null_layers_for_the_fill() {
    use crate::streaming::ChunkGenerator;
    let Some(base) = prepared() else {
        return;
    };
    let assets = Assets::load(&base).unwrap();
    let biomes = crate::biome::Biomes::load(
        &horizon_assets::resource_source::ResourceRoot::directory(base.to_path_buf()),
    )
    .unwrap();
    let item_data =
        crate::test_resources::item_data(&base, DataLoadOrigin::ServerDataLoad).unwrap();
    let terrain_resources = crate::vanillagen::TerrainResources {
        data: None,
        dir: &base.join("worldgen"),
        registry: assets.registry,
        biomes: &biomes,
        item_data: &item_data,
    };
    let flat = horizon_world::worldgen::generator::FlatSettings {
        layers: vec![
            ("minecraft:bedrock".into(), 1),
            ("minecraft:dirt".into(), 2),
            ("minecraft:snow".into(), 1),
        ],
        biome: "minecraft:plains".into(),
        structure_overrides: Some(vec!["minecraft:strongholds".into()]),
        features: false,
        lakes: false,
    };
    let make = |seed| {
        crate::vanillagen::VanillaTerrain::for_flat(
            terrain_resources,
            &Default::default(),
            seed,
            &flat,
            true,
        )
        .unwrap()
    };
    let stronghold = |seed| {
        make(seed)
            .locate(&["minecraft:stronghold".to_owned()], [0, 64, 0], 100)
            .expect("a stronghold")
    };
    assert_eq!(stronghold(1234), stronghold(-987654321));
    // `moveBelowSeaLevel(-63, ChunkGenerator.getMinY() = 0, ..)`: the sea level less 10 is far
    // below the room's height above 0, so the stronghold's top lands at `ySpan + 1` above Y 0
    // (with the level's -64 it would sit 64 blocks lower). The start's box is inflated by 12
    // for the terrain adaptation: the pieces span Y 2 to 36, the box -10 to 48.
    let (_, at) = stronghold(1234);
    let starts = make(1234).starts_at(at[0] >> 4, at[2] >> 4);
    let start = starts
        .iter()
        .find(|s| s.name == "minecraft:stronghold")
        .expect("the stronghold start");
    assert!(
        start.bounding_box.min[1] >= -12,
        "the stronghold sits below the generator's Y 0: {:?}",
        start.bounding_box
    );
    let terrain = make(1234);
    let mut worker = terrain.worker();
    let chunk = terrain.finish(terrain.proto(&mut worker, 3, 3));
    let name = |y| {
        let id = chunk.block(3 * 16 + 5, y, 3 * 16 + 7);
        assets.registry.state(id).unwrap().name.clone()
    };
    // The snow layer (index 3, y -61) is filled later, not by the terrain.
    assert_eq!(
        [-64, -63, -62, -61].map(name),
        [
            "minecraft:bedrock",
            "minecraft:dirt",
            "minecraft:dirt",
            "minecraft:air"
        ]
        .map(String::from)
    );
}

/// `structure_overrides` is a lenient field: a list naming a structure set that does not exist
/// fails to decode and counts as absent, so every set generates.
#[test]
#[ignore = "daily"]
fn a_superflat_structure_override_naming_no_set_lets_every_set_generate() {
    let Some(base) = prepared() else {
        return;
    };
    let assets = Assets::load(&base).unwrap();
    let biomes = crate::biome::Biomes::load(
        &horizon_assets::resource_source::ResourceRoot::directory(base.to_path_buf()),
    )
    .unwrap();
    let item_data =
        crate::test_resources::item_data(&base, DataLoadOrigin::ServerDataLoad).unwrap();
    let terrain_resources = crate::vanillagen::TerrainResources {
        data: None,
        dir: &base.join("worldgen"),
        registry: assets.registry,
        biomes: &biomes,
        item_data: &item_data,
    };
    let flat = horizon_world::worldgen::generator::FlatSettings {
        layers: vec![
            ("minecraft:bedrock".into(), 1),
            ("minecraft:dirt".into(), 2),
            ("minecraft:grass_block".into(), 1),
        ],
        biome: "minecraft:plains".into(),
        structure_overrides: Some(vec!["minecraft:village".into()]),
        features: false,
        lakes: false,
    };
    let terrain = crate::vanillagen::VanillaTerrain::for_flat(
        terrain_resources,
        &Default::default(),
        1234,
        &flat,
        true,
    )
    .unwrap();
    assert!(
        terrain
            .locate(&["minecraft:stronghold".to_owned()], [0, 64, 0], 12)
            .is_some()
    );
    assert!(
        terrain
            .locate(&["minecraft:village_plains".to_owned()], [0, 64, 0], 12)
            .is_some()
    );
}
