use super::*;
#[path = "mod_content_item_metadata_tests.rs"]
mod item_metadata_tests;
use crate::{assets::Assets, blocks::BlockFacts, crafting::CraftingRemainder, items::Items};
use horizon_mod_package::manifest::JarRef;
use horizon_world::native_load_context::DataLoadOrigin;
#[path = "mod_content_biome_modifier_tests.rs"]
mod biome_modifier_tests;
#[path = "mod_content_block_entity_tests.rs"]
mod block_entity_tests;
#[path = "mod_content_block_state_properties_tests.rs"]
mod block_state_properties_tests;
#[path = "mod_content_block_state_tables_tests.rs"]
mod block_state_tables_tests;
#[path = "mod_content_exploration_maps_tests.rs"]
mod exploration_maps_tests;
#[path = "mod_content_farmland_tests.rs"]
mod farmland_tests;
#[path = "mod_content_jar_tests.rs"]
mod jar_tests;
#[path = "mod_content_shapes_tests.rs"]
mod shapes_tests;
#[path = "mod_content_structure_modifier_tests.rs"]
mod structure_modifier_tests;
#[path = "mod_content_tuple_shapes_tests.rs"]
mod tuple_shapes_tests;
#[path = "mod_content_worldgen_tests.rs"]
mod worldgen_tests;
/// A Java jar's assets are a pack layer under the mod's own files, in mod
/// order under the later mods. Passive assets stay in the jar, and the files
/// every pack stacks (atlases here) merge across vanilla, the jars and the
/// mods in that same order.
#[test]
fn a_mods_jars_layer_under_its_files_in_mod_order() {
    use std::io::Write as _;
    let scratch = std::env::temp_dir().join(format!("horizon-mod-jars-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let base = scratch.join("base");
    small_base(&base);
    // `build` reads the base's registry tables even for a plan with no
    // blocks: a minimal facts pair satisfies it without the full export.
    fs::write(
        base.join("facts.json"),
        r#"{"states": [["minecraft:stone"]], "defaults": {"minecraft:stone": 0}}"#,
    )
    .unwrap();
    fs::write(
        base.join("gameplay-facts.json"),
        r#"{"states": [{"item": "minecraft:stone"}]}"#,
    )
    .unwrap();
    let vanilla_path = "assets/minecraft/textures/block/vanilla_path.png";
    fs::create_dir_all(base.join(vanilla_path).parent().unwrap()).unwrap();
    fs::write(base.join(vanilla_path), b"vanilla").unwrap();
    fs::create_dir_all(base.join("assets/minecraft/atlases")).unwrap();
    fs::write(
        base.join("assets/minecraft/atlases/blocks.json"),
        r#"{"sources": [{"type": "directory", "source": "block"}]}"#,
    )
    .unwrap();
    let atlas = |source: &str| format!("{{\"sources\": [{{\"source\": \"{source}\"}}]}}");
    // A jar in the store as the launcher names it: `<sha512>.jar`, with
    // entries in any namespace (a jar is not fenced as a mod's files are).
    let jar = |name: &str, entries: &[(&str, String)]| {
        let sha512 = format!("{}{}", "1".repeat(127), name.chars().last().unwrap());
        let file = scratch.join(format!("jars/{sha512}.jar"));
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        let mut zip = zip::ZipWriter::new(fs::File::create(&file).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        for (entry, text) in entries
            .iter()
            .chain(std::iter::once(&("Example.class", "dummy".to_owned())))
        {
            zip.start_file(*entry, options).unwrap();
            zip.write_all(text.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
        JarRef::new(name.into(), "1.0".into(), sha512).unwrap()
    };
    let ported = |dir: &str, id: &str, files: &[(&str, String)], jars: Vec<JarRef>| {
        let mut m = folder_mod(
            &scratch.join(dir),
            &files
                .iter()
                .map(|(name, text)| (*name, text.as_str()))
                .chain(std::iter::once((
                    "horizon-mod.toml",
                    manifest(id, "").as_str(),
                )))
                .collect::<Vec<_>>(),
        );
        m.jars = jars;
        m
    };
    fn own<'a>(path: &'a str, text: &str) -> (&'a str, String) {
        (path, text.to_owned())
    }
    let atlas_file = |source: &str| ("assets/minecraft/atlases/blocks.json", atlas(source));
    let jar_a = jar(
        "porteda",
        &[
            own("assets/alpha/own_and_jar.png", "jar-a"),
            own("assets/alpha/own_only.png", "jar-a"),
            own(vanilla_path, "jar-a-mc"),
            atlas_file("jar_a"),
        ],
    );
    let jar_b = jar(
        "portedb",
        &[
            own("assets/alpha/own_and_jar.png", "jar-b"),
            own("assets/beta/only_jar.png", "jar-b-only"),
            atlas_file("jar_b"),
        ],
    );
    let alpha = ported(
        "alpha",
        "alpha",
        &[
            own("assets/alpha/own_and_jar.png", "mod-a"),
            own("assets/alpha/own_only.png", "mod-a"),
            atlas_file("mod_a"),
        ],
        vec![jar_a.clone()],
    );
    let beta = ported(
        "beta",
        "beta",
        &[own("assets/beta/own.png", "mod-b"), atlas_file("mod_b")],
        vec![jar_b.clone()],
    );
    let root = scratch.join("root");
    let mods = [alpha, beta];
    let warnings = build_fixture(&base, &scratch.join("jars"), &mods, &root).unwrap();
    let resources = resolved_resources(
        &base,
        root.clone(),
        &scratch.join("jars"),
        &mods,
        scratch.join("cache"),
    )
    .unwrap();
    assert!(!root.join("assets/alpha").exists());
    assert!(!root.join("assets/beta").exists());
    let mut listed = warnings.clone();
    listed.sort();
    assert_eq!(
        listed,
        [
            "mod alpha: jar porteda also provides assets/alpha/own_and_jar.png; the mod's own file wins",
            "mod alpha: jar porteda also provides assets/alpha/own_only.png; the mod's own file wins",
        ]
    );
    // A mod's own files win over its jars; a later mod's jar wins over an
    // earlier mod's own file; a jar wins over vanilla.
    assert_eq!(
        resources
            .source()
            .read("assets/alpha/own_only.png")
            .unwrap(),
        b"mod-a"
    );
    assert_eq!(
        resources
            .source()
            .read("assets/alpha/own_and_jar.png")
            .unwrap(),
        b"jar-b"
    );
    assert_eq!(resources.source().read(vanilla_path).unwrap(), b"jar-a-mc");
    assert_eq!(
        resources.source().read("assets/beta/only_jar.png").unwrap(),
        b"jar-b-only"
    );
    // The stacked files merge across vanilla, the jars and the mods in the
    // same order, and no root Java bytecode leaves the jar.
    let sources: Vec<_> = serde_json::from_slice::<serde_json::Value>(
        &resources
            .source()
            .read("assets/minecraft/atlases/blocks.json")
            .unwrap(),
    )
    .unwrap()["sources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["source"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(sources, ["block", "jar_a", "mod_a", "jar_b", "mod_b"]);
    assert!(!root.join("Example.class").exists());
    // A mod without its own files still layers its jar, under an earlier
    // mod's own files.
    let root = scratch.join("root2");
    let mods = [
        ported(
            "alpha2",
            "alpha",
            &[own("assets/alpha/own.png", "mod-a")],
            vec![jar_a],
        ),
        ported("beta-empty", "beta", &[], vec![jar_b]),
    ];
    build_fixture(&base, &scratch.join("jars"), &mods, &root).unwrap();
    let resources = resolved_resources(
        &base,
        root.clone(),
        &scratch.join("jars"),
        &mods,
        scratch.join("cache"),
    )
    .unwrap();
    assert!(resources.source().contains("assets/alpha/own.png"));
    assert!(!root.join("assets/alpha").exists());
    assert_eq!(
        resources.source().read("assets/beta/only_jar.png").unwrap(),
        b"jar-b-only"
    );
    let _ = fs::remove_dir_all(&scratch);
}

/// An absent original refuses publication before either the tables or the
/// decoded pack root exists.
#[test]
fn a_missing_jar_refuses_the_whole_load_before_decoded_output() {
    let scratch =
        std::env::temp_dir().join(format!("horizon-mod-jar-missing-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let base = scratch.join("base");
    small_base(&base);
    // `build` reads the base's registry tables even for a plan with no
    // blocks: a minimal facts pair satisfies it without the full export.
    fs::write(
        base.join("facts.json"),
        r#"{"states": [["minecraft:stone"]], "defaults": {"minecraft:stone": 0}}"#,
    )
    .unwrap();
    fs::write(
        base.join("gameplay-facts.json"),
        r#"{"states": [{"item": "minecraft:stone"}]}"#,
    )
    .unwrap();
    let shared = "assets/alpha/textures/block/own.png";
    let mut m = folder_mod(
        &scratch.join("alpha"),
        &[
            ("horizon-mod.toml", &manifest("alpha", "")),
            (shared, "own"),
        ],
    );
    m.jars = vec![JarRef::new("porteda".into(), "1.0".into(), "2".repeat(128)).unwrap()];
    let root = scratch.join("root");
    let error = build_fixture(
        &base,
        &scratch.join("jars"),
        std::slice::from_ref(&m),
        &root,
    )
    .unwrap_err();
    let missing = error
        .downcast_ref::<horizon_mod_package::manifest::MissingJar>()
        .unwrap();
    assert_eq!(missing.owners, ["alpha"]);
    assert_eq!(missing.jar, m.jars[0]);
    assert_eq!(missing.path, m.jars[0].cache_path(&scratch.join("jars")));
    assert!(!root.exists());
    assert!(!scratch.join("root-tables").exists());
    let _ = fs::remove_dir_all(&scratch);
}

use std::sync::Arc;

#[test]
fn state_light_compiles_in_final_order_and_relights_across_a_chunk_boundary() {
    use crate::{
        chunk::Chunk,
        light::{self, LightChanges, LightTables},
    };
    use horizon_world::{
        level::GameplayStateSource,
        registry::{BlockRegistry, BlockStateId},
    };
    use std::collections::HashMap;
    let declaration = r#"{"template":"minecraft:test","item":false,"properties":{"light":3},
      "state_properties":{"mode":{"type":"enum","values":["off","dim","bright"]}},
      "outline_context_shapes":{"item":{"properties":["mode"],"values":{
        "dim":[[0,0,0,0.5,1,1]]},"default":[]}},
      "state_light":{"properties":["mode","axis"],"values":{
        "dim,x":8,"dim,y":9,"bright,x":15,"bright,y":12},"default":0}}"#;
    let (base, mods) = block_base_of(
        "state-light-propagation",
        &[declaration],
        r#"{"states":[
      ["minecraft:air",false,false,"",{"id":"minecraft:air"}],
      ["minecraft:test",false,false,"",{"id":"minecraft:test","properties":{"axis":"x"}}],
      ["minecraft:test",false,false,"",{"id":"minecraft:test","properties":{"axis":"y"}}]],
      "defaults":{"minecraft:air":0,"minecraft:test":1}}"#,
    );
    let plan = Plan::new(&base, &mods).unwrap();
    let mut facts: J = read_json(&base.join("facts.json")).unwrap();
    let mut gameplay: J = read_json(&base.join("gameplay-facts.json")).unwrap();
    let Some(J::Arr(rows)) = gameplay.at("states") else {
        unreachable!()
    };
    for (i, row) in rows.iter_mut().enumerate() {
        row.set("id", J::Num(i.into()));
        row.set("emission", J::Num(if i == 0 { 0 } else { 3 }.into()));
    }
    plan.json("facts.json", &mut facts).unwrap();
    plan.json("gameplay-facts.json", &mut gameplay).unwrap();
    fs::write(
        base.join("gameplay-facts.json"),
        serde_json::to_vec(&gameplay).unwrap(),
    )
    .unwrap();
    let registry =
        BlockRegistry::from_facts_json(serde_json::to_vec(&facts).unwrap().as_slice()).unwrap();
    let blocks: BTreeSet<_> = (0..registry.len())
        .map(|i| {
            registry
                .state(BlockStateId(i as u32))
                .unwrap()
                .name
                .as_str()
        })
        .collect();
    let blocks: Vec<_> = blocks
        .iter()
        .map(|name| serde_json::json!({"name":name,"class":["Block"]}))
        .collect();
    let behaviours = serde_json::json!({
        "states": vec![serde_json::json!([0,0]);registry.len()],"blocks":blocks
    });
    let (facts, projection) =
        BlockFacts::load_with_projection(&base, &registry, &behaviours, false).unwrap();
    assert_eq!(
        facts.get(BlockStateId(1)).emission,
        3,
        "the template is unchanged"
    );
    let tables = LightTables::new(&facts);
    let mut by_state = BTreeMap::new();
    for i in 3..registry.len() as u32 {
        let state = registry.state(BlockStateId(i)).unwrap();
        let key = (
            state.properties["axis"].as_str(),
            state.properties["mode"].as_str(),
        );
        let expected = match key {
            (_, "off") => 0,
            ("x", "dim") => 8,
            ("y", "dim") => 9,
            ("x", "bright") => 15,
            ("y", "bright") => 12,
            _ => unreachable!(),
        };
        assert_eq!(facts.get(BlockStateId(i)).emission, expected);
        assert_eq!(
            projection[i as usize].unsigned("emission"),
            u64::from(expected)
        );
        assert_eq!(tables.emission(i), expected);
        let item_outline = &plan.blocks[0].context_shapes["outline_item"][(i - 3) as usize];
        assert_eq!(
            item_outline.bounds().map(|bounds| bounds.max(Axis::X)),
            (key.1 == "dim").then_some(0.5)
        );
        by_state.insert(key, i);
    }
    let off = by_state[&("x", "off")];
    let dim = by_state[&("x", "dim")];
    let bright = by_state[&("x", "bright")];
    assert!(tables.differs(dim, bright));
    assert!(tables.differs(bright, off));
    let mut world = HashMap::new();
    for z in -1..=1 {
        for x in -1..=1 {
            let mut chunk = Chunk::empty(x, z);
            if (x, z) == (0, 0) {
                chunk.set(15, 64, 8, dim);
            }
            world.insert((x, z), Arc::new(chunk));
        }
    }
    let lit: Vec<_> = world
        .keys()
        .map(|&(x, z)| {
            let region = std::array::from_fn(|dz| {
                std::array::from_fn(|dx| {
                    world
                        .get(&(x + dx as i32 - 1, z + dz as i32 - 1))
                        .map(AsRef::as_ref)
                })
            });
            let mut chunk = (*world[&(x, z)]).clone();
            light::apply_lighting(&mut chunk, light::light_chunk(&region, &tables, false));
            chunk.lit = true;
            ((x, z), Arc::new(chunk))
        })
        .collect();
    world = lit.into_iter().collect();
    let level = |world: &HashMap<(i32, i32), Arc<Chunk>>, x: i32| {
        world[&(x >> 4, 0)].light(x & 15, 64, 8) & 15
    };
    assert_eq!((level(&world, 15), level(&world, 16)), (8, 7));
    for (state, expected) in [(bright, 15u8), (off, 0u8)] {
        Arc::make_mut(world.get_mut(&(0, 0)).unwrap()).set(15, 64, 8, state);
        let mut changed = LightChanges::default();
        light::relight(
            &mut world,
            &tables,
            false,
            &[(15, 64, 8)],
            false,
            &mut changed,
        );
        assert_eq!(
            (level(&world, 15), level(&world, 16)),
            (expected, expected.saturating_sub(1))
        );
        assert!(!changed.is_empty());
    }
    fs::remove_dir_all(base.parent().unwrap()).unwrap();
}

#[test]
fn state_light_rejects_unknown_unreachable_and_uncovered_state_tuples() {
    for (i, spec) in [
        r#"{"property":"missing","default":0}"#,
        r#"{"property":"axis","values":{"x":1}}"#,
        r#"{"property":"axis","values":{"x":1,"z":2},"default":0}"#,
    ]
    .iter()
    .enumerate()
    {
        let declaration = format!(r#"{{"template":"minecraft:test","state_light":{spec}}}"#);
        let (base, mods) = block_base(&format!("state-light-selection-{i}"), &[&declaration]);
        assert!(Plan::new(&base, &mods).is_err(), "{spec}");
        fs::remove_dir_all(base.parent().unwrap()).unwrap();
    }
    for (i, light) in ["", r#", "properties":{"light":4}"#].iter().enumerate() {
        let declaration = format!(r#"{{"template":"minecraft:test"{light}}}"#);
        let (base, mods) = block_base(&format!("state-light-absent-{i}"), &[&declaration]);
        let plan = Plan::new(&base, &mods).unwrap();
        let mut doc: J =
            serde_json::from_str(r#"{"states":[{"emission":6},{"emission":7}]}"#).unwrap();
        plan.json("gameplay-facts.json", &mut doc).unwrap();
        let expected = if i == 0 { vec![6, 7] } else { vec![4, 4] };
        let doc = serde_json::to_value(doc).unwrap();
        assert_eq!(doc["states"][2]["emission"], expected[0]);
        assert_eq!(doc["states"][3]["emission"], expected[1]);
        fs::remove_dir_all(base.parent().unwrap()).unwrap();
    }
}

#[test]
fn state_light_from_native_declarations_invalidates_the_content_cache() {
    let (base, mut mods) = block_base(
        "state-light-cache",
        &[r#"{"template":"minecraft:test","state_light":{"default":8}}"#],
    );
    fs::write(base.join("horizon-resources.json"), b"{}").unwrap();
    let initial = stamp(&base, &base.join("jars"), &mods, &fixture_inventory(&mods)).unwrap();
    let declaration = &mut mods[0].content.blocks[0];
    declaration.state_light.as_mut().unwrap().default = Some(15);
    assert_ne!(
        stamp(&base, &base.join("jars"), &mods, &fixture_inventory(&mods)).unwrap(),
        initial
    );
    mods[0].content.blocks[0]
        .state_light
        .as_mut()
        .unwrap()
        .default = Some(8);
    assert_eq!(
        stamp(&base, &base.join("jars"), &mods, &fixture_inventory(&mods)).unwrap(),
        initial
    );
    fs::remove_dir_all(base.parent().unwrap()).unwrap();
}

#[test]
#[ignore = "daily"]
fn block_aliases_keep_the_original_item_in_placement_context() {
    use horizon_core::{
        geometry::{Direction, Vec3},
        position::BlockPos,
        random::LegacyRandom,
    };
    use horizon_mod_package::content::{Block, Item};
    use horizon_world::{
        level::{
            Behaviours, LevelAccess, LevelEvent, PlaceContext, PlaceOutcome, PlacementContext,
            Ticks,
        },
        loot::PackOverlay,
        registry::BlockStateId,
    };
    use std::{cell::RefCell, collections::HashMap};
    let Some(base) = std::env::var_os("HORIZON_TEST_RESOURCES")
        .map(PathBuf::from)
        .or_else(prepared)
    else {
        return;
    };
    let scratch =
        std::env::temp_dir().join(format!("horizon-alias-context-{}", std::process::id()));
    let mut content = Content::declared("ruby");
    content
        .declare(
            &[Block {
                id: "ruby:frame".into(),
                template: "minecraft:stone".into(),
                item: false,
                ..Default::default()
            }],
            &["empty", "filled"].map(|name| Item {
                id: format!("ruby:{name}"),
                template: "minecraft:stick".into(),
                block: Some("ruby:frame".into()),
                ..Default::default()
            }),
        )
        .unwrap();
    build_fixture(
        &base,
        Path::new(""),
        &[ContentMod {
            id: "ruby".into(),
            content,
            jars: Vec::new(),
        }],
        &scratch,
    )
    .unwrap();
    let registry = horizon_world::level::data::load_registry(&scratch).unwrap();
    let facts = BlockFacts::load(&scratch, &registry).unwrap();
    let defaults = horizon_world::item_defaults::read(&scratch).unwrap();
    let items = Items::load(&scratch, &registry, &facts, &PackOverlay::NONE, &defaults).unwrap();
    let b = Behaviours::load(&registry, &scratch).unwrap();
    let target = registry.default_state("ruby:frame").unwrap();
    struct Level {
        air: BlockStateId,
        blocks: HashMap<BlockPos, BlockStateId>,
        random: LegacyRandom,
        asked: RefCell<Vec<String>>,
        client: bool,
    }
    impl horizon_world::level::BlockEntityNeighbors for Level {}
    impl LevelAccess for Level {
        fn block_state(&self, p: BlockPos) -> BlockStateId {
            self.blocks.get(&p).copied().unwrap_or(self.air)
        }
        fn set_raw(&mut self, p: BlockPos, s: BlockStateId) -> Option<BlockStateId> {
            Some(self.blocks.insert(p, s).unwrap_or(self.air))
        }
        fn is_loaded(&self, _: BlockPos) -> bool {
            true
        }
        fn random(&mut self) -> &mut LegacyRandom {
            &mut self.random
        }
        fn game_time(&self) -> i64 {
            0
        }
        fn emit(&mut self, _: LevelEvent) {}
        fn is_client_side(&self) -> bool {
            self.client
        }
        fn before_state_for_placement(
            &self,
            _: BlockStateId,
            c: &PlacementContext,
            _: horizon_world::fluid::FluidState,
            _: &mut Option<Option<BlockStateId>>,
        ) {
            self.asked.borrow_mut().push(c.item.clone());
        }
    }
    for client in [false, true] {
        let mut level = Level {
            air: facts.air,
            blocks: HashMap::new(),
            random: LegacyRandom::new(1),
            asked: RefCell::default(),
            client,
        };
        for (i, name) in ["ruby:empty", "ruby:filled"].iter().enumerate() {
            assert_eq!(items.get(name).unwrap().block, Some(target));
            assert_eq!(b.item_binding(name).unwrap().block, b.block(target));
            let pos = BlockPos::new(i as i32 * 2, 70, 0);
            let ctx = PlaceContext {
                clicked_pos: pos,
                face: Direction::Up,
                hit: Vec3::new(f64::from(pos.x) + 0.5, 70.0, 0.5),
                inside: false,
                yaw: 0.0,
                pitch: 0.0,
                sneaking: false,
                item: (*name).into(),
                can_use_game_master_blocks: false,
            };
            assert!(
                matches!(b.place_block(&mut level,&mut Ticks::default(),&ctx),PlaceOutcome::Placed{state,..} if state==target)
            );
            assert_eq!(level.asked.borrow().last().map(String::as_str), Some(*name));
        }
    }
    fs::remove_dir_all(scratch).unwrap();
}

#[test]
fn block_compiles_ordinary_bindings_without_changing_components() {
    use horizon_mod_package::content::Item;
    let (base, mut mods) = block_base(
        "place-block-bindings",
        &[r#"{"template":"minecraft:test","item":false}"#],
    );
    let item = |id: &str, target: Option<&str>| Item {
        id: id.into(),
        template: "minecraft:test".into(),
        block: target.map(str::to_owned),
        ..Default::default()
    };
    mods[0]
        .content
        .declare(
            &[],
            &[
                item("testmod:empty", Some("testmod:test")),
                item("testmod:filled", Some("testmod:test")),
                item("testmod:vanilla", Some("minecraft:test")),
                item("testmod:plain", None),
            ],
        )
        .unwrap();
    let plan = Plan::new(&base, &mods).unwrap();
    let mut behaviours:J=serde_json::from_str(r#"{"states":[{},{}],"blocks":[],"items":{"minecraft:test":{"class":["Item"],"fields":{}}}}"#).unwrap();
    plan.json("block-behaviours.json", &mut behaviours).unwrap();
    let behaviours = serde_json::to_value(behaviours).unwrap();
    for name in ["empty", "filled", "vanilla"] {
        let binding = &behaviours["items"][format!("testmod:{name}")];
        assert_eq!(binding["class"], serde_json::json!(["BlockItem", "Item"]));
        assert_eq!(
            binding["fields"]["block"],
            if name == "vanilla" {
                "minecraft:test"
            } else {
                "testmod:test"
            }
        );
    }
    assert_eq!(
        behaviours["items"]["testmod:plain"]["class"],
        serde_json::json!(["Item"])
    );
    let mut components:J=serde_json::from_str(r#"{"minecraft:test":{"minecraft:max_stack_size":16,"minecraft:item_model":"minecraft:test"}}"#).unwrap();
    plan.json("items.json", &mut components).unwrap();
    let components = serde_json::to_value(components).unwrap();
    assert_eq!(components["testmod:empty"]["minecraft:max_stack_size"], 16);
    assert_eq!(
        components["testmod:empty"]["minecraft:item_model"],
        "testmod:empty"
    );
    drop(plan);
    for target in ["testmod:missing", "testmod:plain", "minecraft:missing"] {
        mods[0].content.items[0].block = Some(target.into());
        assert!(
            Plan::new(&base, &mods)
                .err()
                .unwrap()
                .to_string()
                .contains("not a block")
        );
    }
    fs::remove_dir_all(base.parent().unwrap()).unwrap();
}

#[test]
fn renames_ids_keys_and_states() {
    let mut r = Rename::new("minecraft:iron_block", "ruby:ruby_block");
    r.states = 10..12;
    r.first = 100;
    assert_eq!(
        r.string("minecraft:iron_block").as_deref(),
        Some("ruby:ruby_block")
    );
    assert_eq!(
        r.string("Block{minecraft:iron_block}").as_deref(),
        Some("Block{ruby:ruby_block}")
    );
    assert_eq!(
        r.string("block.minecraft.iron_block").as_deref(),
        Some("block.ruby.ruby_block")
    );
    assert_eq!(r.string("minecraft:iron_blocks"), None);
    assert_eq!(r.string("block.minecraft.iron_bars"), None);
    assert_eq!(
        (r.id(9), r.id(10), r.id(11), r.id(12)),
        (None, Some(100), Some(101), None)
    );
}

#[test]
fn json_keeps_key_order_and_numbers() {
    let text = r#"{"z":1,"a":[0.6,5.0,-1,{"m":null,"b":true}],"k":"v"}"#;
    let doc: J = serde_json::from_str(text).unwrap();
    assert_eq!(serde_json::to_string(&doc).unwrap(), text);
}

#[test]
fn item_components_patch_the_template_entry() {
    let mut entry: J =
        serde_json::from_str(r#"{"minecraft:max_damage": 250, "minecraft:repairable": {}}"#)
            .unwrap();
    let known: BTreeSet<String> = [
        "minecraft:max_damage",
        "minecraft:repairable",
        "minecraft:food",
    ]
    .map(str::to_owned)
    .into();
    let patch = |text: &str| -> Vec<(String, J)> {
        let J::Obj(o) = serde_json::from_str(text).unwrap() else {
            unreachable!()
        };
        o
    };
    patch_components(
        &mut entry,
        &patch(r#"{"minecraft:max_damage": 900, "!minecraft:repairable": {}, "minecraft:food": {"nutrition": 4}}"#),
        &known,
    )
    .unwrap();
    assert_eq!(
        serde_json::to_string(&entry).unwrap(),
        r#"{"minecraft:max_damage":900,"minecraft:food":{"nutrition":4}}"#
    );
    for typo in [r#"{"minecraft:fod": {}}"#, r#"{"!minecraft:fod": {}}"#] {
        let e = patch_components(&mut entry, &patch(typo), &known).unwrap_err();
        assert_eq!(e, "minecraft:fod is not a known component type");
    }
}

#[test]
fn creative_tabs_keep_vanilla_items_missing_from_search() {
    let tab = horizon_mod_package::content::CreativeTab {
        id: "test:restricted".into(),
        title: "Restricted".into(),
        icon: "minecraft:barrier".into(),
        items: [
            "minecraft:barrier",
            "minecraft:command_block",
            "minecraft:written_book",
            "minecraft:knowledge_book",
        ]
        .map(str::to_owned)
        .into(),
    };
    let plan = Plan {
        blocks: Vec::new(),
        items: Vec::new(),
        vanilla_items: tab.items.iter().cloned().collect(),
        effects: Vec::new(),
        fluids: Vec::new(),
        entity_types: Vec::new(),
        block_entity_types: Vec::new(),
        block_entity_factories: BTreeMap::new(),
        menu_types: Vec::new(),
        vanilla_contexts: BTreeMap::new(),
        warnings: Vec::new(),
        states: 0,
        defaults: BTreeMap::new(),
        sounds: BTreeMap::new(),
        tabs: vec![&tab],
        names: BTreeMap::new(),
    };
    let mut doc: J = serde_json::from_str(
        r#"{"tabs":[{"type":"SEARCH","items":["{count:1,id:\"minecraft:stone\"}"]}]}"#,
    )
    .unwrap();
    plan.creative_tabs(&mut doc).unwrap();
    let doc: serde_json::Value = serde_json::to_value(doc).unwrap();
    assert_eq!(
        doc["tabs"][1]["items"],
        serde_json::json!([
            "{count:1,id:\"minecraft:barrier\"}",
            "{count:1,id:\"minecraft:command_block\"}",
            "{count:1,id:\"minecraft:written_book\"}",
            "{count:1,id:\"minecraft:knowledge_book\"}",
        ])
    );
}

/// A block item's patch goes over its template block's item, and a type no
/// vanilla item has fails the build.
#[test]
fn block_items_patch_their_template_item() {
    let scratch = std::env::temp_dir().join(format!("horizon-block-item-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let base = scratch.join("base");
    fs::create_dir_all(&base).unwrap();
    fs::write(
        base.join("facts.json"),
        r#"{"states": [["minecraft:iron_block"]], "defaults": {"minecraft:iron_block": 0}}"#,
    )
    .unwrap();
    fs::write(
        base.join("gameplay-facts.json"),
        r#"{"states": [{"item": "minecraft:iron_block"}]}"#,
    )
    .unwrap();
    let items = r##"{"minecraft:iron_block": {"minecraft:max_stack_size": 64,
        "minecraft:item_name": {"translate": "block.minecraft.iron_block"}},
        "minecraft:netherite_block": {"minecraft:damage_resistant": {"types": "#minecraft:is_fire"}}}"##;
    let block = |dir: &str, components: &str| {
        folder_mod(
            &scratch.join(dir),
            &[
                ("horizon-mod.toml", &manifest("ruby", "")),
                (
                    "data/ruby/horizon/block/ruby_block.json",
                    &format!(
                        r#"{{"template": "minecraft:iron_block", "components": {components}}}"#
                    ),
                ),
            ],
        )
    };
    let mods = [block(
        "ruby",
        r##"{"minecraft:max_stack_size": 16, "minecraft:damage_resistant": {"types": "#minecraft:is_fire"}}"##,
    )];
    let mut doc: J = serde_json::from_str(items).unwrap();
    Plan::new(&base, &mods)
        .unwrap()
        .json("items.json", &mut doc)
        .unwrap();
    let doc: serde_json::Value = serde_json::to_value(&doc).unwrap();
    assert_eq!(
        doc["ruby:ruby_block"],
        serde_json::json!({"minecraft:max_stack_size": 16,
            "minecraft:item_name": {"translate": "block.ruby.ruby_block"},
            "minecraft:damage_resistant": {"types": "#minecraft:is_fire"}})
    );
    assert_eq!(doc["minecraft:iron_block"]["minecraft:max_stack_size"], 64);
    let mods = [block("typo", r#"{"minecraft:fod": {}}"#)];
    let mut doc: J = serde_json::from_str(items).unwrap();
    let e = Plan::new(&base, &mods)
        .unwrap()
        .json("items.json", &mut doc)
        .unwrap_err();
    assert_eq!(
        e.to_string(),
        "ruby:ruby_block: minecraft:fod is not a known component type"
    );
    fs::remove_dir_all(scratch).unwrap();
}

#[test]
fn a_mods_atlas_sources_follow_vanillas() {
    let vanilla = r#"{"sources": [{"type": "directory", "source": "block", "prefix": "block/"}]}"#;
    let more = [serde_json::json!({"type": "directory", "source": "mek_chemical"})];
    let merged: serde_json::Value = serde_json::from_slice(
        &crate::packs::append_atlas_sources(Some(vanilla), &more, "a").unwrap(),
    )
    .unwrap();
    let sources: Vec<_> = merged["sources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["source"].as_str().unwrap())
        .collect();
    assert_eq!(sources, ["block", "mek_chemical"]);
}

/// Registry IDs and state IDs must not depend on package discovery order.
#[test]
fn allocated_ids_follow_mod_and_entry_order_after_vanilla() {
    let scratch = std::env::temp_dir().join(format!("horizon-mod-order-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let base = scratch.join("base");
    fs::create_dir_all(&base).unwrap();
    fs::write(
        base.join("facts.json"),
        r#"{"states": [["minecraft:iron_block"], ["minecraft:iron_block"], ["minecraft:stone"]],
            "defaults": {"minecraft:iron_block": 0, "minecraft:stone": 2}}"#,
    )
    .unwrap();
    fs::write(
        base.join("gameplay-facts.json"),
        r#"{"states": [{"item":"minecraft:iron_block"}, {"item":"minecraft:iron_block"},
            {"item":"minecraft:stone"}]}"#,
    )
    .unwrap();
    fs::write(
        base.join("items.json"),
        r#"{"minecraft:stone": {}, "minecraft:iron_block": {}}"#,
    )
    .unwrap();
    fs::write(
        base.join("registries.json"),
        r#"{"minecraft:item":{"entries":{"minecraft:stone":{"protocol_id":4},
            "minecraft:iron_block":{"protocol_id":9}}},
            "minecraft:block":{"entries":{"minecraft:stone":{"protocol_id":7},
            "minecraft:iron_block":{"protocol_id":11}}},
            "minecraft:mob_effect":{"entries":{"minecraft:speed":{"protocol_id":0},
            "minecraft:luck":{"protocol_id":25}}},
            "minecraft:fluid":{"entries":{"minecraft:empty":{"protocol_id":0},
            "minecraft:flowing_water":{"protocol_id":1},"minecraft:water":{"protocol_id":2},
            "minecraft:flowing_lava":{"protocol_id":3},"minecraft:lava":{"protocol_id":4}}},
            "minecraft:block_entity_type":{"entries":{"minecraft:chest":{"protocol_id":1}}}}"#,
    )
    .unwrap();
    fs::create_dir_all(base.join("assets/minecraft/blockstates")).unwrap();
    fs::write(
        base.join("assets/minecraft/blockstates/iron_block.json"),
        "{}",
    )
    .unwrap();
    fs::write(base.join("assets/minecraft/blockstates/stone.json"), "{}").unwrap();
    let ruby = folder_mod(
        &scratch.join("ruby"),
        &[
            ("horizon-mod.toml", &manifest("ruby", "")),
            (
                "data/ruby/horizon/block/zblock.json",
                r#"{"template":"minecraft:stone"}"#,
            ),
            (
                "data/ruby/horizon/item/aitem.json",
                r#"{"template":"minecraft:stone"}"#,
            ),
            (
                "data/ruby/horizon/mob_effect/glow.json",
                r#"{"category":"beneficial","color":16711680}"#,
            ),
            (
                "data/ruby/horizon/fluid/honey.json",
                r#"{"sprite":"ruby:block/h"}"#,
            ),
        ],
    );
    let alpha = folder_mod(
        &scratch.join("alpha"),
        &[
            ("horizon-mod.toml", &manifest("alpha", "")),
            (
                "data/alpha/horizon/block/zblock.json",
                r#"{"template":"minecraft:iron_block"}"#,
            ),
            (
                "data/alpha/horizon/item/aitem.json",
                r#"{"template":"minecraft:stone"}"#,
            ),
            (
                "data/alpha/horizon/mob_effect/zap.json",
                r#"{"category":"harmful","color":255}"#,
            ),
            (
                "data/alpha/horizon/mob_effect/dull.json",
                r#"{"category":"neutral","color":0}"#,
            ),
            (
                "data/alpha/horizon/fluid/molten/tin.json",
                r#"{"sprite":"alpha:block/s"}"#,
            ),
        ],
    );
    let mut mods = vec![ruby, alpha];
    mods[0].content.block_entity_types.push(BlockEntityType {
        id: "ruby:machine".into(),
        blocks: vec!["alpha:zblock".into()],
    });
    mods[1].content.block_entity_types.push(BlockEntityType {
        id: "alpha:machine".into(),
        blocks: vec!["ruby:zblock".into()],
    });
    for index in 0..2 {
        let plan = Plan::new(&base, &mods).unwrap();
        let blocks: Vec<_> = plan
            .blocks
            .iter()
            .map(|block| {
                (
                    block.decl.id.as_str(),
                    block.rename.first,
                    block.rename.states.len(),
                )
            })
            .collect();
        assert_eq!(blocks, [("alpha:zblock", 3, 2), ("ruby:zblock", 5, 1)]);
        let items: Vec<_> = plan
            .items
            .iter()
            .map(|item| item.rename.to.as_str())
            .collect();
        assert_eq!(
            items,
            ["alpha:aitem", "alpha:zblock", "ruby:aitem", "ruby:zblock"]
        );

        // Exercise the same registry rewrite the game and server read.
        let mut registry: J = serde_json::from_str(
            r#"{"minecraft:item":{"entries":{"minecraft:stone":{"protocol_id":4},
                "minecraft:iron_block":{"protocol_id":9}}},
                "minecraft:block":{"entries":{"minecraft:stone":{"protocol_id":7},
                "minecraft:iron_block":{"protocol_id":11}}},
                "minecraft:mob_effect":{"entries":{"minecraft:speed":{"protocol_id":0},
                "minecraft:luck":{"protocol_id":25}}},
                "minecraft:fluid":{"entries":{"minecraft:empty":{"protocol_id":0},
                "minecraft:flowing_water":{"protocol_id":1},"minecraft:water":{"protocol_id":2},
                "minecraft:flowing_lava":{"protocol_id":3},"minecraft:lava":{"protocol_id":4}}},
                "minecraft:block_entity_type":{"entries":{"minecraft:chest":{"protocol_id":1}}}}"#,
        )
        .unwrap();
        plan.json("registries.json", &mut registry).unwrap();
        let registry: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&registry).unwrap()).unwrap();
        let id = |kind: &str, name: &str| {
            registry[kind]["entries"][name]["protocol_id"]
                .as_u64()
                .unwrap()
        };
        assert_eq!(id("minecraft:item", "minecraft:stone"), 4);
        assert_eq!(id("minecraft:item", "minecraft:iron_block"), 9);
        assert_eq!(id("minecraft:item", "alpha:aitem"), 10);
        assert_eq!(id("minecraft:item", "alpha:zblock"), 11);
        assert_eq!(id("minecraft:item", "ruby:aitem"), 12);
        assert_eq!(id("minecraft:item", "ruby:zblock"), 13);
        assert_eq!(id("minecraft:block", "minecraft:stone"), 7);
        assert_eq!(id("minecraft:block", "minecraft:iron_block"), 11);
        assert_eq!(id("minecraft:block", "alpha:zblock"), 12);
        assert_eq!(id("minecraft:block", "ruby:zblock"), 13);
        // Only declared block entity types get IDs, sorted independently of blocks.
        assert_eq!(id("minecraft:block_entity_type", "minecraft:chest"), 1);
        assert_eq!(id("minecraft:block_entity_type", "alpha:machine"), 2);
        assert_eq!(id("minecraft:block_entity_type", "ruby:machine"), 3);
        assert!(registry["minecraft:block_entity_type"]["entries"]["alpha:zblock"].is_null());
        assert!(registry["minecraft:block_entity_type"]["entries"]["ruby:zblock"].is_null());

        assert_eq!(id("minecraft:mob_effect", "minecraft:luck"), 25);
        assert_eq!(id("minecraft:mob_effect", "alpha:dull"), 26);
        assert_eq!(id("minecraft:mob_effect", "alpha:zap"), 27);
        assert_eq!(id("minecraft:mob_effect", "ruby:glow"), 28);
        // Each mod fluid after vanilla's, by id: its source, then its flowing form.
        assert_eq!(id("minecraft:fluid", "minecraft:lava"), 4);
        assert_eq!(id("minecraft:fluid", "alpha:molten/tin"), 5);
        assert_eq!(id("minecraft:fluid", "alpha:flowing_molten/tin"), 6);
        assert_eq!(id("minecraft:fluid", "ruby:honey"), 7);
        assert_eq!(id("minecraft:fluid", "ruby:flowing_honey"), 8);

        // `/effect` and the mob_effect arguments check ids against this table.
        let mut commands: J = serde_json::from_str(
            r#"{"blocks":[],"items":[],"effects":["minecraft:speed"],
                "by_registry":{"minecraft:mob_effect":{"ids":["minecraft:speed"]},
                "minecraft:fluid":{"ids":["minecraft:water"]}}}"#,
        )
        .unwrap();
        plan.json("command-registries.json", &mut commands).unwrap();
        let commands: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&commands).unwrap()).unwrap();
        let want = ["minecraft:speed", "alpha:dull", "alpha:zap", "ruby:glow"];
        assert_eq!(commands["effects"], serde_json::json!(want));
        assert_eq!(
            commands["by_registry"]["minecraft:mob_effect"]["ids"],
            serde_json::json!(want)
        );
        // Fluid tag files and fluid arguments check ids against this table.
        assert_eq!(
            commands["by_registry"]["minecraft:fluid"]["ids"],
            serde_json::json!([
                "minecraft:water",
                "alpha:molten/tin",
                "alpha:flowing_molten/tin",
                "ruby:honey",
                "ruby:flowing_honey"
            ])
        );
        drop(plan);

        let output = scratch.join(format!("output-{index}"));
        build_fixture(&base, Path::new(""), &mods, &output).unwrap();
        let built: serde_json::Value =
            serde_json::from_slice(&fs::read(output.join("registries.json")).unwrap()).unwrap();
        assert_eq!(
            built, registry,
            "the built root must use the tested allocation"
        );
        assert_eq!(
            fs::read_to_string(output.join(BLOCK_ENTITY_TYPES)).unwrap(),
            r#"{"alpha:zblock":"ruby:machine","ruby:zblock":"alpha:machine"}"#
        );
        mods.reverse();
    }
    fs::remove_dir_all(scratch).unwrap();
}

fn prepared() -> Option<PathBuf> {
    if let Some(root) = std::env::var_os("HORIZON_TEST_RESOURCES") {
        let root = PathBuf::from(root);
        assert!(
            root.join("gameplay-facts.json").is_file(),
            "HORIZON_TEST_RESOURCES is not prepared"
        );
        return Some(root);
    }
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    [
        manifest.join("../../.local/horizon-client/26.3"),
        manifest.join("../../../../.local/horizon-client/26.3"),
    ]
    .into_iter()
    .find(|root| root.join("gameplay-facts.json").exists())
}

fn content_files(
    resources: &horizon_assets::resource_source::ResourceRoot,
) -> Vec<(String, Vec<Vec<u8>>)> {
    let content = crate::datapack::DataPack::from_source(
        "horizon-content".into(),
        resources.data_changes().clone(),
    );
    crate::datapack::DataStore::load(&[&content])
        .files
        .into_iter()
        .collect()
}

/// Exercise relative prepared roots even when the data lives outside this checkout.
fn relative_base(base: &Path) -> PathBuf {
    let base = std::path::absolute(base).unwrap();
    let current = std::env::current_dir().unwrap();
    let target = base.components().collect::<Vec<_>>();
    let origin = current.components().collect::<Vec<_>>();
    let shared = target
        .iter()
        .zip(&origin)
        .take_while(|(a, b)| a == b)
        .count();
    if shared == 0 {
        return base;
    }
    let mut relative = PathBuf::new();
    for _ in &origin[shared..] {
        relative.push("..");
    }
    for component in &target[shared..] {
        relative.push(component.as_os_str());
    }
    relative
}

/// A content mod folder: a full block, a door and an item-less glass from
/// templates, items, a recipe, a vanilla tag addition and a language file.
fn ruby_mod(dir: &Path) -> ContentMod {
    ruby_mod_with_pistons(dir, false)
}

fn ruby_mod_with_pistons(dir: &Path, pistons: bool) -> ContentMod {
    let files: [(&str, &str); 11] = [
        (
            "horizon-mod.toml",
            "id = \"ruby\"\nname = \"Ruby\"\nversion = \"0.1.0\"\napi = \"0.2\"\nside = \"both\"\ncapabilities = [\"registries\"]\n",
        ),
        (
            "data/ruby/horizon/block/ruby_block.json",
            r##"{"template": "minecraft:iron_block", "properties": {"hardness": 4.0, "light": 3,
                "sounds": "minecraft:amethyst_block", "map_color": 28, "friction": 0.8,
                "random_ticks": true, "occludes": false, "destroy_overlay": false},
                "components": {"minecraft:max_stack_size": 16,
                    "minecraft:damage_resistant": {"types": "#minecraft:is_fire"}}}"##,
        ),
        (
            "data/ruby/horizon/block/ruby_door.json",
            r#"{"template": "minecraft:oak_door", "properties": {"push_reaction": "IMMOVEABLE"}}"#,
        ),
        (
            "data/ruby/horizon/block/ruby_glass.json",
            r#"{"template": "minecraft:glass", "item": false,
                "properties": {"render_shape": "ENTITYBLOCK_ANIMATED"}}"#,
        ),
        (
            "data/ruby/horizon/item/ruby.json",
            r#"{"template": "minecraft:diamond", "max_stack_size": 16, "rarity": "rare", "block": "ruby:ruby_glass"}"#,
        ),
        (
            "data/ruby/horizon/item/ruby_sword.json",
            r#"{"template": "minecraft:iron_sword", "block": "minecraft:wheat", "components": {"minecraft:max_damage": 900,
                "minecraft:attribute_modifiers": [{"type": "minecraft:attack_damage",
                    "id": "minecraft:base_attack_damage", "amount": 7.0,
                    "operation": "add_value", "slot": "mainhand"}],
                "!minecraft:repairable": {}}}"#,
        ),
        (
            "data/ruby/horizon/item/ruby_stew.json",
            r#"{"template": "minecraft:mushroom_stew", "crafting_remainder": "minecraft:bowl"}"#,
        ),
        (
            "data/ruby/recipe/ruby_from_stew.json",
            r#"{"type": "minecraft:crafting_shapeless", "category": "misc",
                "ingredients": ["ruby:ruby_stew"], "result": {"id": "ruby:ruby"}}"#,
        ),
        // Carried-over tags name blocks left out and other mods' ids too, and
        // 1.21.1's vanilla tags 26.3 renamed (`concrete_powders`).
        (
            "data/minecraft/tags/block/mineable/pickaxe.json",
            r##"{"values": ["ruby:ruby_block", "ruby:left_out", "othermod:thing",
                {"id": "othermod:optional", "required": false}, "#c:storage_blocks/ruby",
                "#minecraft:concrete_powder"]}"##,
        ),
        (
            "data/c/tags/item/gems/ruby.json",
            r#"{"values": ["ruby:ruby", "othermod:ruby"]}"#,
        ),
        (
            "assets/ruby/lang/en_us.json",
            r#"{"block.ruby.ruby_block": "Block of Ruby"}"#,
        ),
    ];
    for (name, text) in files {
        let path = dir.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    if pistons {
        for (name, text) in [
            ("z_stone", r#"{"template":"minecraft:stone"}"#),
            (
                "z_stuck",
                r#"{"template":"minecraft:stone","properties":{"push_reaction":"IMMOVEABLE"}}"#,
            ),
        ] {
            fs::write(
                dir.join(format!("data/ruby/horizon/block/{name}.json")),
                text,
            )
            .unwrap();
        }
    }
    let package = horizon_mod_package::Package::read(dir).unwrap();
    ContentMod {
        id: "ruby".into(),
        content: Content::read(dir, &package.manifest).unwrap(),
        jars: Vec::new(),
    }
}

/// The overlay over the real prepared resources: every loader reads it, the
/// new blocks behave like their templates with the overrides, and vanilla's
/// states are where they were.
#[test]
#[ignore = "daily"]
fn content_root_loads_with_every_table() {
    let Some(base) = prepared() else {
        return;
    };
    let scratch = std::env::temp_dir().join(format!("horizon-mod-content-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let m = ruby_mod_with_pistons(&scratch.join("ruby"), true);
    // The base as a relative path (cargo runs tests in the crate's folder):
    // the overlay's links still resolve.
    // Readers of the root hold its lock; Windows removes no folder with an open file.
    {
        let relative = relative_base(&base);
        let root = resource_root_fixture(
            &relative,
            Path::new(""),
            std::slice::from_ref(&m),
            &scratch.join("cache"),
        )
        .unwrap();
        assert_ne!(root, base);
        let resources = resolved_resources(
            &base,
            root.clone(),
            Path::new(""),
            std::slice::from_ref(&m),
            scratch.join("cache"),
        )
        .unwrap();

        let files = content_files(&resources);
        let overlay = PackOverlay::new(&files);
        let vanilla = Assets::load(&base).unwrap();
        let assets = Assets::load(&root).unwrap();
        let (old, new) = (vanilla.registry, assets.registry);
        let n = old.len() as u32;
        let iron = old.default_state("minecraft:iron_block").unwrap();
        let door = old.default_state("minecraft:oak_door").unwrap();
        let door_first = (0..n)
            .map(BlockStateId)
            .find(|&s| old.state(s).unwrap().name == "minecraft:oak_door")
            .unwrap();
        // Vanilla keeps its ids; blocks come after, sorted: ruby_block, ruby_door, ruby_glass.
        assert_eq!(new.len(), old.len() + 1 + 64 + 1 + 2);
        assert_eq!(new.default_state("minecraft:iron_block"), Some(iron));
        let ruby = new.default_state("ruby:ruby_block").unwrap();
        assert_eq!(ruby, BlockStateId(n));
        let ruby_door = new.default_state("ruby:ruby_door").unwrap();
        assert_eq!(ruby_door, BlockStateId(n + 1 + (door.0 - door_first.0)));
        assert_eq!(
            new.default_state("ruby:ruby_glass"),
            Some(BlockStateId(n + 65))
        );
        // Network ids: the new items and blocks follow vanilla's in the registries report, which
        // the game and its server both read (a missing one travels as air).
        let report = |root: &Path| {
            horizon_protocol::registry::Builtin::load(&root.join("registries.json")).unwrap()
        };
        let (vanilla_ids, ids) = (report(&base), report(&root));
        let item = |b: &horizon_protocol::registry::Builtin, name: &str| {
            b.get("minecraft:item").and_then(|r| r.id(name))
        };
        let vanilla_items = (0..)
            .take_while(|&i| vanilla_ids.get("minecraft:item").unwrap().name(i).is_some())
            .count() as i32;
        assert_eq!(
            item(&ids, "minecraft:stone"),
            item(&vanilla_ids, "minecraft:stone")
        );
        let ruby_item = item(&ids, "ruby:ruby_block").unwrap();
        assert!(ruby_item >= vanilla_items, "{ruby_item} < {vanilla_items}");
        assert_eq!(
            ids.get("minecraft:item").unwrap().name(ruby_item),
            Some("ruby:ruby_block")
        );
        assert!(item(&ids, "ruby:ruby_glass").is_none(), "no item declared");
        assert!(
            ids.get("minecraft:block")
                .unwrap()
                .id("ruby:ruby_glass")
                .is_some()
        );

        let old_facts = BlockFacts::load(&base, old).unwrap();
        let facts = BlockFacts::load(&root, new).unwrap();
        let shared_json: serde_json::Value = serde_json::from_reader(std::io::BufReader::new(
            fs::File::open(root.join("block-behaviours.json")).unwrap(),
        ))
        .unwrap();
        let (shared_facts, shared_behaviours) =
            BlockFacts::load_with_behaviours(&root, new, &shared_json).unwrap();
        let independent_behaviours = horizon_world::level::Behaviours::load(new, &root).unwrap();
        let (deferred_facts, gameplay) =
            BlockFacts::load_with_projection(&root, new, &shared_json, true).unwrap();
        let deferred_behaviours = horizon_world::level::Behaviours::load_with_gameplay(
            new,
            &root,
            &shared_json,
            &gameplay,
        )
        .unwrap();
        assert_eq!(shared_facts.states.len(), facts.states.len());
        // `random_ticks` makes the block tick randomly although its template (iron) never does; the
        // sections' counts read the same flag (`random_tick_candidate`).
        assert!(independent_behaviours.is_randomly_ticking(ruby));
        assert!(independent_behaviours.random_tick_candidate(ruby));
        assert!(!independent_behaviours.is_randomly_ticking(iron));
        assert!(deferred_behaviours.is_randomly_ticking(ruby));
        for state in (n + 1)..(n + 65) {
            assert_eq!(
                independent_behaviours.facts(BlockStateId(state)).push,
                horizon_world::level::PushReaction::Immoveable
            );
        }
        assert_eq!(
            independent_behaviours.facts(ruby).push,
            independent_behaviours.facts(iron).push
        );
        // `occludes: false` (noOcclusion): the iron-template block occludes nothing, iron still does.
        assert!(!facts.get(ruby).can_occlude && !facts.get(ruby).solid_render);
        // Its light dampening is a non-solid full block's, as leaves' and spawners' are (iron's is 15).
        assert_eq!(
            (facts.get(ruby).dampening, facts.get(iron).dampening),
            (1, 15)
        );
        assert!(facts.get(iron).can_occlude && facts.get(iron).solid_render);
        // `destroy_overlay: false` hides the crack on the declared block only.
        assert!(!facts.get(ruby).destroy_overlay && facts.get(iron).destroy_overlay);
        let definitions: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join("render-definitions.json")).unwrap())
                .unwrap();
        assert_eq!(
            definitions["ruby:ruby_block"]["can_occlude"], false,
            "faces beside it are drawn"
        );
        assert_eq!(definitions["minecraft:iron_block"]["can_occlude"], true);
        // `render_shape` leaves the glass's model out of the chunk mesh; the block keeps the template's.
        assert_eq!(
            facts.get(BlockStateId(n + 65)).render_shape,
            crate::blocks::RenderShape::Animated
        );
        assert_eq!(
            facts.get(ruby).render_shape,
            crate::blocks::RenderShape::Model
        );
        for state in [ruby, ruby_door, BlockStateId(n + 65)] {
            assert_eq!(shared_facts.get(state).hardness, facts.get(state).hardness);
            assert_eq!(
                deferred_facts.get(state).hardness,
                facts.get(state).hardness
            );
            assert_eq!(deferred_facts.get(state).classes, facts.get(state).classes);
            assert_eq!(
                deferred_behaviours.facts(state).flags,
                independent_behaviours.facts(state).flags
            );
            assert_eq!(
                deferred_behaviours.data(state).sounds.place,
                independent_behaviours.data(state).sounds.place
            );
            assert_eq!(shared_facts.get(state).emission, facts.get(state).emission);
            assert_eq!(shared_facts.get(state).classes, facts.get(state).classes);
            assert_eq!(
                shared_behaviours.facts(state).flags,
                independent_behaviours.facts(state).flags,
            );
            assert_eq!(
                shared_behaviours.data(state).sounds.place,
                independent_behaviours.data(state).sounds.place,
            );
        }

        let (a, b) = (old_facts.get(iron), facts.get(ruby));
        assert_eq!((b.hardness, b.emission, b.friction), (4.0, 3, 0.8));
        assert_eq!(b.explosion_resistance, a.explosion_resistance);
        assert_eq!(b.outline, a.outline);
        assert_eq!(b.classes, a.classes);
        assert_eq!(&*b.item, "ruby:ruby_block");
        assert_eq!(b.map_color, 28);
        let amethyst = old_facts.get(old.default_state("minecraft:amethyst_block").unwrap());
        assert_eq!(b.sounds.break_sound, amethyst.sounds.break_sound);
        for i in 0..64 {
            let (v, m) = (
                old_facts.get(BlockStateId(door_first.0 + i)),
                facts.get(BlockStateId(n + 1 + i)),
            );
            assert_eq!(
                (v.outline.clone(), v.hardness, v.classes.clone()),
                (m.outline.clone(), m.hardness, m.classes.clone())
            );
            assert_eq!(&*m.name, "ruby:ruby_door");
        }
        let glass = facts.get(BlockStateId(n + 65));
        assert_eq!(&*glass.item, "minecraft:air");
        // Vanilla's own rows are untouched.
        assert_eq!(facts.get(iron).hardness, a.hardness);
        assert_eq!(&*facts.get(iron).name, "minecraft:iron_block");

        let defaults = horizon_world::item_defaults::read(&root).unwrap();
        let items = Items::load(&root, new, &facts, &overlay, &defaults).unwrap();
        item_metadata_tests::assert_metadata(&root, &items, &defaults, ruby, n);
        let binding = independent_behaviours.item_binding("ruby:ruby").unwrap();
        assert_eq!(
            binding.block,
            independent_behaviours.block(BlockStateId(n + 65))
        );
        // Exercise the same binding as server placement, including its emitted sound.
        {
            use horizon_core::{
                geometry::{Direction, Vec3},
                position::BlockPos,
                random::LegacyRandom,
            };
            use horizon_world::level::{
                LevelAccess, LevelEvent, PlaceContext, PlaceOutcome, Ticks,
            };
            struct TestLevel {
                blocks: std::collections::HashMap<BlockPos, BlockStateId>,
                air: BlockStateId,
                random: LegacyRandom,
                events: Vec<LevelEvent>,
                time: i64,
            }
            impl horizon_world::level::BlockEntityNeighbors for TestLevel {}
            impl LevelAccess for TestLevel {
                fn block_state(&self, pos: BlockPos) -> BlockStateId {
                    *self.blocks.get(&pos).unwrap_or(&self.air)
                }
                fn set_raw(&mut self, pos: BlockPos, state: BlockStateId) -> Option<BlockStateId> {
                    Some(self.blocks.insert(pos, state).unwrap_or(self.air))
                }
                fn is_loaded(&self, _: BlockPos) -> bool {
                    true
                }
                fn random(&mut self) -> &mut LegacyRandom {
                    &mut self.random
                }
                fn game_time(&self) -> i64 {
                    self.time
                }
                fn emit(&mut self, event: LevelEvent) {
                    self.events.push(event);
                }
            }
            let floor = BlockPos::new(0, 0, 0);
            let wheat = new.default_state("minecraft:wheat").unwrap();
            let mut level = TestLevel {
                blocks: [(floor, iron)].into_iter().collect(),
                air: new.default_state("minecraft:air").unwrap(),
                random: LegacyRandom::new(1),
                events: Vec::new(),
                time: 0,
            };
            let ctx = PlaceContext {
                clicked_pos: floor,
                face: Direction::Up,
                hit: Vec3::new(0.5, 1.0, 0.5),
                inside: false,
                yaw: 0.0,
                pitch: 0.0,
                sneaking: false,
                item: "ruby:ruby_sword".into(),
                can_use_game_master_blocks: false,
            };
            let denied = independent_behaviours.place_block(&mut level, &mut Ticks::new(1), &ctx);
            assert!(matches!(denied, PlaceOutcome::Failed));
            assert!(level.events.is_empty());
            level
                .blocks
                .insert(floor, new.default_state("minecraft:farmland").unwrap());
            let placed = independent_behaviours.place_block(&mut level, &mut Ticks::new(1), &ctx);
            assert!(matches!(placed, PlaceOutcome::Placed { state, .. } if state == wheat));
            assert!(level.events.iter().any(|e| matches!(e, LevelEvent::Sound { sound, .. } if &**sound == "minecraft:item.crop.plant")));

            // Exercise the real piston caller with both stone-based content blocks.
            let piston = new
                .resolve(&horizon_world::chunk::BlockState {
                    name: "minecraft:piston".into(),
                    properties: [
                        ("facing".into(), "east".into()),
                        ("extended".into(), "false".into()),
                    ]
                    .into_iter()
                    .collect(),
                })
                .unwrap();
            let power = new.default_state("minecraft:redstone_block").unwrap();
            for (name, moves) in [("ruby:z_stone", true), ("ruby:z_stuck", false)] {
                let block = new.default_state(name).unwrap();
                let base = BlockPos::new(0, 1, 0);
                let front = BlockPos::new(1, 1, 0);
                let destination = BlockPos::new(2, 1, 0);
                let mut level = TestLevel {
                    blocks: Default::default(),
                    air: new.default_state("minecraft:air").unwrap(),
                    random: LegacyRandom::new(1),
                    events: Vec::new(),
                    time: 0,
                };
                let mut ticks = Ticks::new(0);
                for x in -4..4 {
                    for z in -4..4 {
                        ticks
                            .add_empty_chunk(horizon_core::position::ChunkPos::new(x, z))
                            .unwrap();
                    }
                }
                for (pos, state) in [
                    (base, piston),
                    (front, block),
                    (BlockPos::new(-1, 1, 0), power),
                ] {
                    independent_behaviours.set_block(
                        &mut level,
                        &mut ticks,
                        pos,
                        state,
                        horizon_world::level::flags::UPDATE_ALL,
                    );
                }
                for _ in 0..4 {
                    level.time += 1;
                    independent_behaviours.tick_level_blocks(&mut level, &mut ticks);
                    independent_behaviours.tick_block_entities(&mut level, &mut ticks);
                }
                assert_eq!(
                    new.state(level.block_state(base)).unwrap().properties["extended"],
                    if moves { "true" } else { "false" }
                );
                assert_eq!(
                    level.block_state(if moves { destination } else { front }),
                    block
                );
            }
        }
        let sword = &items.items["ruby:ruby_sword"];
        assert_eq!(sword.block, new.default_state("minecraft:wheat"));
        assert_eq!(items.items["minecraft:iron_block"].block, Some(iron));
        let iron_sword = &items.items["minecraft:iron_sword"];
        assert_eq!((sword.max_damage, iron_sword.max_damage), (900, 250));
        assert_eq!(sword.modifiers.len(), 1);
        assert_eq!(sword.modifiers[0].amount, 7.0);
        assert_eq!(sword.tool.len(), iron_sword.tool.len());
        // The stew leaves its bowl in the grid; vanilla's stew leaves none.
        let stew = &items.items["ruby:ruby_stew"];
        assert_eq!(stew.crafting_remainder.as_deref(), Some("minecraft:bowl"));
        assert_eq!(
            items.items["minecraft:mushroom_stew"].crafting_remainder,
            None
        );
        let recipes = crate::crafting::Recipes::load(
            &root,
            &items,
            &overlay,
            &crate::test_resources::server_load(),
        )
        .unwrap();
        let grid = [
            Some(crate::items::ItemStack::new("ruby:ruby_stew", 1)),
            None,
            None,
            None,
        ];
        let (id, result) = recipes
            .find(crate::test_resources::server_loop_context(), &grid, 2)
            .unwrap();
        assert_eq!(&*result.item, "ruby:ruby");
        let rest = recipes.remainders(id, &grid, 2, &|s| items.crafting_remainder(s));
        assert_eq!(rest[0].as_ref().map(|s| &*s.item), Some("minecraft:bowl"));
        assert!(!items.items.contains_key("ruby:ruby_glass"));
        let pickaxe = &items.block_tags["minecraft:mineable/pickaxe"];
        assert!(pickaxe.contains("ruby:ruby_block"));
        assert!(!pickaxe.contains("ruby:left_out") && !pickaxe.contains("othermod:thing"));
        assert!(pickaxe.contains("minecraft:stone"));
        // Convention tags load, with the ids nobody registers dropped.
        assert_eq!(items.item_tag_order["c:gems/ruby"], ["ruby:ruby"]);

        crate::collisions::load(&root, new).unwrap();
        crate::models::load_visibility(&root, new).unwrap();
        crate::light::LightTables::load(&root, new, &facts).unwrap();
        crate::motion::MotionResources::load(&root, &assets).unwrap();
        horizon_world::worldgen::generator::GeneratorData::load(&root.join("worldgen"), new)
            .unwrap();
        assert!(resources.source().contains("assets/ruby/lang/en_us.json"));
        // Blocks without a blockstate of their own take their template's.
        assert_eq!(
            resources
                .source()
                .read("assets/ruby/blockstates/ruby_door.json")
                .unwrap(),
            fs::read(base.join("assets/minecraft/blockstates/oak_door.json")).unwrap()
        );

        // The base root was only read.
        assert!(
            fs::symlink_metadata(base.join("facts.json"))
                .unwrap()
                .is_file()
        );
    }
    fs::remove_dir_all(&scratch).unwrap();
}

#[test]
fn refuses_unknown_templates_and_vanilla_ids() {
    let Some(base) = prepared() else {
        return;
    };
    let scratch = std::env::temp_dir().join(format!("horizon-mod-refuse-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let mut m = ruby_mod(&scratch.join("ruby"));
    m.content.blocks[0].template = "minecraft:not_a_block".into();
    let e = build_fixture(
        &base,
        Path::new(""),
        std::slice::from_ref(&m),
        &scratch.join("out"),
    )
    .unwrap_err();
    assert!(e.to_string().contains("is not a block"), "{e}");
    m.content.blocks[0].template = "minecraft:iron_block".into();
    let gem = m
        .content
        .items
        .iter_mut()
        .find(|i| i.id == "ruby:ruby")
        .unwrap();
    gem.block = Some("minecraft:not_a_block".into());
    let e = build_fixture(
        &base,
        Path::new(""),
        std::slice::from_ref(&m),
        &scratch.join("out"),
    )
    .unwrap_err();
    assert!(
        e.to_string()
            .contains("block \"minecraft:not_a_block\" is not a block"),
        "{e}"
    );
    m.content
        .items
        .iter_mut()
        .find(|i| i.id == "ruby:ruby")
        .unwrap()
        .block = Some("ruby:ruby_glass".into());
    let stew = m
        .content
        .items
        .iter_mut()
        .find(|i| i.id == "ruby:ruby_stew");
    stew.unwrap().crafting_remainder = Some("minecraft:bowll".into());
    let e = build_fixture(
        &base,
        Path::new(""),
        std::slice::from_ref(&m),
        &scratch.join("out"),
    )
    .unwrap_err();
    assert!(
        e.to_string()
            .contains(r#"crafting_remainder "minecraft:bowll" is not an item"#),
        "{e}"
    );
    fs::remove_dir_all(&scratch).unwrap();
}

#[test]
fn refuses_more_states_than_a_world_holds() {
    let Some(base) = prepared() else {
        return;
    };
    let scratch = std::env::temp_dir().join(format!("horizon-mod-states-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let mut m = ruby_mod(&scratch.join("ruby"));
    // Redstone wire has 1296 states: 810 copies pass 2^20 with any game.
    let mut wire = m.content.blocks[0].clone();
    wire.template = "minecraft:redstone_wire".into();
    for i in 0..810 {
        let mut b = wire.clone();
        b.id = format!("ruby:wire_{i}");
        m.content.blocks.push(b);
    }
    let e = build_fixture(
        &base,
        Path::new(""),
        std::slice::from_ref(&m),
        &scratch.join("out"),
    )
    .unwrap_err();
    assert!(
        e.to_string()
            .contains("more than the 1048576 a world can hold"),
        "{e}"
    );
    assert!(e.to_string().contains("the mods: ruby "), "{e}");
    fs::remove_dir_all(&scratch).unwrap();
}

#[test]
fn builds_past_sixteen_bit_states() {
    let Some(base) = prepared() else {
        return;
    };
    let scratch = std::env::temp_dir().join(format!("horizon-mod-wide-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let mut m = ruby_mod(&scratch.join("ruby"));
    // 40 redstone wires: 51,840 states, past 65,536 with vanilla's 35,723.
    let mut wire = m.content.blocks[0].clone();
    wire.template = "minecraft:redstone_wire".into();
    for i in 0..40 {
        let mut b = wire.clone();
        b.id = format!("ruby:wire_{i}");
        m.content.blocks.push(b);
    }
    build_fixture(
        &base,
        Path::new(""),
        std::slice::from_ref(&m),
        &scratch.join("out"),
    )
    .unwrap();
    fs::remove_dir_all(&scratch).unwrap();
}

#[test]
fn no_content_keeps_the_base_root() {
    let base = Path::new("/nonexistent/base");
    assert_eq!(
        resource_root_fixture(base, Path::new(""), &[], Path::new("/nonexistent/cache")).unwrap(),
        base
    );
}

use horizon_world::loot::PackOverlay;
use horizon_world::registry::BlockStateId;

#[test]
#[ignore = "daily"]
fn shared_behaviour_document_matches_independent_loaders() {
    let Some(root) = prepared() else { return };
    let assets = Assets::load(&root).expect("assets");
    let doc: serde_json::Value = serde_json::from_reader(std::io::BufReader::new(
        std::fs::File::open(root.join("block-behaviours.json")).unwrap(),
    ))
    .unwrap();
    let original = BlockFacts::load(&root, assets.registry).unwrap();
    let (shared, b) = BlockFacts::load_with_behaviours(&root, assets.registry, &doc).unwrap();
    let (deferred_facts, projection) =
        BlockFacts::load_with_projection(&root, assets.registry, &doc, true).unwrap();
    let deferred = horizon_world::level::Behaviours::load_with_gameplay(
        assets.registry,
        &root,
        &doc,
        &projection,
    )
    .unwrap();
    assert_eq!(deferred_facts.states.len(), shared.states.len());
    assert_eq!(original.states.len(), shared.states.len());
    for (a, b) in original.states.iter().zip(&shared.states) {
        assert_eq!(a.classes, b.classes);
        assert_eq!(a.legacy_solid, b.legacy_solid);
        assert_eq!(a.sturdy_full, b.sturdy_full);
        assert_eq!(a.emission, b.emission);
        assert_eq!(a.dampening, b.dampening);
        assert_eq!(a.hardness, b.hardness);
        assert_eq!(a.sounds.volume, b.sounds.volume);
        assert_eq!(a.sounds.pitch, b.sounds.pitch);
    }
    let a = horizon_world::level::Behaviours::load(assets.registry, &root).unwrap();
    for id in 0..assets.registry.len() {
        let state = horizon_world::registry::BlockStateId(id as u32);
        let (a_state, b_state) = (a.facts(state), b.facts(state));
        let deferred_state = deferred.facts(state);
        assert_eq!(b_state.flags, deferred_state.flags);
        assert_eq!(b_state.emission, deferred_state.emission);
        assert_eq!(b_state.dampening, deferred_state.dampening);
        assert_eq!(b_state.hardness, deferred_state.hardness);
        assert_eq!(b_state.push, deferred_state.push);
        assert_eq!(a_state.flags, b_state.flags);
        assert_eq!(a_state.sturdy, b_state.sturdy);
        assert_eq!(a_state.weak, b_state.weak);
        assert_eq!(a_state.strong, b_state.strong);
        assert_eq!(a_state.fluid, b_state.fluid);
        assert_eq!(a_state.emission, b_state.emission);
        assert_eq!(a_state.dampening, b_state.dampening);
        assert_eq!(a_state.hardness, b_state.hardness);
        assert_eq!(a_state.push, b_state.push);
        assert_eq!(a_state.instrument, b_state.instrument);
        assert_eq!(a_state.shape, b_state.shape);
        let (a_sound, b_sound) = (&a.data(state).sounds, &b.data(state).sounds);
        assert_eq!(a_sound.volume, b_sound.volume);
        assert_eq!(a_sound.pitch, b_sound.pitch);
        assert_eq!(a_sound.place, b_sound.place);
        assert_eq!(a_sound.break_, b_sound.break_);
        assert_eq!(a_sound.fall, b_sound.fall);
    }
}

#[cfg(unix)]
#[test]
#[ignore = "daily"]
fn independent_reloads_read_changed_behaviour_json_and_report_invalid_files() {
    let Some(base) = prepared() else { return };
    let scratch = std::env::temp_dir().join(format!("horizon-json-reloads-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    fs::create_dir_all(&scratch).unwrap();
    // Keep the remaining resource tree intact without copying a huge prepared client.
    for entry in fs::read_dir(&base).unwrap() {
        let entry = entry.unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(entry.path(), scratch.join(entry.file_name())).unwrap();
    }
    let assets = Assets::load(&base).unwrap();
    let registry = assets.registry;
    let original = fs::read(base.join("block-behaviours.json")).unwrap();
    fs::remove_file(scratch.join("block-behaviours.json")).unwrap();
    fs::write(scratch.join("block-behaviours.json"), &original).unwrap();
    // Independent facts and behaviours can load first, followed by paired data.
    let independent_first = BlockFacts::load(&scratch, registry).unwrap();
    let independent_world_first =
        horizon_world::level::Behaviours::load(registry, &scratch).unwrap();
    let (facts, paired_world) = BlockFacts::load_pair(&scratch, registry).unwrap();
    assert_eq!(facts.states.len(), registry.len());
    for state in [0, 1, registry.len() - 1] {
        let state = horizon_world::registry::BlockStateId(state as u32);
        assert_eq!(
            facts.get(state).classes,
            independent_first.get(state).classes
        );
        assert_eq!(
            facts.get(state).sounds.place,
            independent_first.get(state).sounds.place
        );
        assert_eq!(
            paired_world.facts(state).flags,
            independent_world_first.facts(state).flags
        );
    }

    // Override one exported state bit at the same path and verify that a later
    // independent and paired read sees it while the original resource root does not.
    let mut changed: serde_json::Value = serde_json::from_slice(&original).unwrap();
    let original_bits = changed["states"][0][1].as_u64().unwrap();
    changed["states"][0][1] = (original_bits ^ 1).into();
    fs::write(
        scratch.join("block-behaviours.json"),
        serde_json::to_vec(&changed).unwrap(),
    )
    .unwrap();
    let changed_pair = BlockFacts::load_pair(&scratch, registry).unwrap();
    let changed_independent = BlockFacts::load(&scratch, registry).unwrap();
    let base_facts = BlockFacts::load(&base, registry).unwrap();
    let state = horizon_world::registry::BlockStateId(0);
    assert_ne!(
        changed_pair.0.get(state).legacy_solid,
        base_facts.get(state).legacy_solid
    );
    assert_eq!(
        changed_pair.0.get(state).legacy_solid,
        changed_independent.get(state).legacy_solid
    );
    // The world loader sees the same overridden legacy solidity flag.
    let changed_world = horizon_world::level::Behaviours::load(registry, &scratch).unwrap();
    assert_eq!(
        changed_world.facts(state).flags & 1 != 0,
        changed_pair.0.get(state).legacy_solid
    );

    fs::write(scratch.join("block-behaviours.json"), b"{").unwrap();
    let paired_error = BlockFacts::load_pair(&scratch, registry)
        .err()
        .unwrap()
        .to_string();
    let independent_error = BlockFacts::load(&scratch, registry)
        .err()
        .unwrap()
        .to_string();
    assert_eq!(paired_error, independent_error);
    assert!(horizon_world::level::Behaviours::load(registry, &scratch).is_err());

    fs::write(scratch.join("block-behaviours.json"), b"{}").unwrap();
    assert!(BlockFacts::load_pair(&scratch, registry).is_err());
    assert!(BlockFacts::load(&scratch, registry).is_err());
    assert!(horizon_world::level::Behaviours::load(registry, &scratch).is_err());

    fs::write(scratch.join("block-behaviours.json"), original).unwrap();
    assert_eq!(
        BlockFacts::load_pair(&scratch, registry)
            .unwrap()
            .0
            .states
            .len(),
        registry.len()
    );
    fs::remove_file(scratch.join("gameplay-facts.json")).unwrap();
    fs::write(scratch.join("gameplay-facts.json"), b"{\"states\":[]}").unwrap();
    assert!(BlockFacts::load_pair(&scratch, registry).is_err());
    assert!(BlockFacts::load(&scratch, registry).is_err());
    assert!(horizon_world::level::Behaviours::load(registry, &scratch).is_err());
    fs::write(scratch.join("gameplay-facts.json"), b"{").unwrap();
    assert!(BlockFacts::load_pair(&scratch, registry).is_err());
    assert!(BlockFacts::load(&scratch, registry).is_err());
    assert!(horizon_world::level::Behaviours::load(registry, &scratch).is_err());
    fs::write(
        scratch.join("gameplay-facts.json"),
        fs::read(base.join("gameplay-facts.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        BlockFacts::load_pair(&scratch, registry)
            .unwrap()
            .0
            .states
            .len(),
        registry.len()
    );
    fs::remove_dir_all(&scratch).unwrap();
}

/// A built root is kept for later starts, and built again when a mod's
/// files change.
#[test]
#[ignore = "daily"]
fn content_root_is_reused_until_a_mod_changes() {
    let Some(base) = prepared() else {
        return;
    };
    let scratch = std::env::temp_dir().join(format!("horizon-mod-reuse-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let mut m = ruby_mod(&scratch.join("ruby"));
    let cache = scratch.join("cache");
    let root =
        resource_root_fixture(&base, Path::new(""), std::slice::from_ref(&m), &cache).unwrap();
    // A rebuild starts from an empty folder: this file tells them apart.
    let sentinel = root.join("sentinel");
    fs::write(&sentinel, b"").unwrap();
    assert_eq!(
        resource_root_fixture(&base, Path::new(""), std::slice::from_ref(&m), &cache).unwrap(),
        root
    );
    assert!(sentinel.exists(), "an unchanged mod rebuilt the root");
    let old_tables = fs::read(root.join("facts.json")).unwrap();
    let lang = scratch.join("ruby/assets/ruby/lang/en_us.json");
    fs::write(&lang, r#"{"block.ruby.ruby_block": "Ruby Block"}"#).unwrap();
    let file = fs::File::options().write(true).open(&lang).unwrap();
    file.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(5))
        .unwrap();
    let changed =
        resource_root_fixture(&base, Path::new(""), std::slice::from_ref(&m), &cache).unwrap();
    assert_ne!(changed, root);
    assert!(sentinel.exists(), "a rebuild removed the active root");
    assert_eq!(fs::read(root.join("facts.json")).unwrap(), old_tables);
    let resources = resolved_resources(
        &base,
        changed.clone(),
        Path::new(""),
        std::slice::from_ref(&m),
        cache.clone(),
    )
    .unwrap();
    assert!(
        resources
            .source()
            .read_to_string("assets/ruby/lang/en_us.json")
            .unwrap()
            .contains("Ruby Block")
    );
    assert!(!changed.join("assets/ruby/lang/en_us.json").exists());
    m.content.blocks[0].template = "minecraft:missing_template".into();
    assert!(resource_root_fixture(&base, Path::new(""), std::slice::from_ref(&m), &cache).is_err());
    assert!(
        changed.exists(),
        "a refused rebuild removed the accepted root"
    );
    assert!(sentinel.exists());
    let _ = fs::remove_dir_all(&scratch);
}

/// `horizon:add_to`: only the set, pool and processor list bundles take additions; each is
/// appended after the target's own entries, lower `horizon:priority` first and equal priorities
/// in the order given (mod, then file order); a target nobody defines is skipped.
#[test]
fn additions_append_by_priority_then_order_and_skip_unknown_targets() {
    let pools = "worldgen/template_pools.json";
    let mut additions: Vec<Addition> = [
        r#"{"horizon:add_to": ["mod:pool", "mod:missing"], "elements": ["a"]}"#,
        r#"{"horizon:add_to": "mod:pool", "horizon:priority": 5, "elements": ["b"]}"#,
        r#"{"horizon:add_to": "mod:pool", "elements": ["c"]}"#,
        r#"{"horizon:add_to": "mod:pool", "horizon:priority": 5, "elements": ["d"]}"#,
        r#"{"horizon:add_to": "mod:pool", "horizon:priority": 1000, "elements": ["e"]}"#,
    ]
    .iter()
    .map(|text| Addition::read(pools, text).unwrap().unwrap())
    .collect();
    let mut doc: J = serde_json::from_str(
        &serde_json::json!({"mod:pool": r#"{"elements": ["own"]}"#}).to_string(),
    )
    .unwrap();
    apply_additions(&mut doc, &mut additions).unwrap();
    let Some(J::Str(text)) = doc.get("mod:pool") else {
        panic!("the pool is kept");
    };
    let pool: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(
        pool["elements"],
        serde_json::json!(["own", "b", "d", "a", "c", "e"])
    );
    assert!(doc.get("mod:missing").is_none());
    let feature = r#"{"horizon:add_to": "mod:pool", "elements": ["a"]}"#;
    assert!(
        Addition::read("worldgen/features.json", feature)
            .unwrap()
            .is_none()
    );
}

/// A mod's mob effect gets the network id after vanilla's last, on the game
/// and its server alike, and joins the ids `/effect` and its suggestions
/// check.
#[test]
#[ignore = "daily"]
fn content_root_registers_a_mod_mob_effect() {
    let Some(base) = prepared() else {
        return;
    };
    let scratch = std::env::temp_dir().join(format!("horizon-mod-effect-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let m = folder_mod(
        &scratch.join("glow"),
        &[
            ("horizon-mod.toml", &manifest("glow", "")),
            (
                "data/glow/horizon/mob_effect/radiance.json",
                r#"{"category": "beneficial", "color": 16773120}"#,
            ),
        ],
    );
    let root = resource_root_fixture(
        &base,
        Path::new(""),
        std::slice::from_ref(&m),
        &scratch.join("cache"),
    )
    .unwrap();
    let report = |root: &Path| {
        horizon_protocol::registry::Builtin::load(&root.join("registries.json")).unwrap()
    };
    let (vanilla, ids) = (report(&base), report(&root));
    let vanilla = vanilla.get("minecraft:mob_effect").unwrap();
    let ids = ids.get("minecraft:mob_effect").unwrap();
    let count = (0..).take_while(|&i| vanilla.name(i).is_some()).count() as i32;
    assert_eq!(ids.id("glow:radiance"), Some(count));
    assert_eq!(ids.id("minecraft:speed"), vanilla.id("minecraft:speed"));
    let commands: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("command-registries.json")).unwrap()).unwrap();
    for ids in [
        &commands["effects"],
        &commands["by_registry"]["minecraft:mob_effect"]["ids"],
    ] {
        let ids = ids.as_array().unwrap();
        assert!(ids.contains(&"glow:radiance".into()) && ids.contains(&"minecraft:speed".into()));
    }
    assert!(root.join(crate::content_identity::MARKER).exists());
    fs::remove_dir_all(&scratch).unwrap();
}

/// A mod that adds only an item still moves the item network ids, so a game
/// without it must fail the content check rather than show other items.
#[test]
#[ignore = "daily"]
fn content_root_with_only_an_item_checks_its_identity() {
    let Some(base) = prepared() else {
        return;
    };
    let scratch = std::env::temp_dir().join(format!("horizon-mod-item-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let m = folder_mod(
        &scratch.join("gem"),
        &[
            ("horizon-mod.toml", &manifest("gem", "")),
            (
                "data/gem/horizon/item/opal.json",
                r#"{"template": "minecraft:diamond"}"#,
            ),
        ],
    );
    let root = resource_root_fixture(
        &base,
        Path::new(""),
        std::slice::from_ref(&m),
        &scratch.join("cache"),
    )
    .unwrap();
    let ids = horizon_protocol::registry::Builtin::load(&root.join("registries.json")).unwrap();
    assert!(ids.get("minecraft:item").unwrap().id("gem:opal").is_some());
    assert!(root.join(crate::content_identity::MARKER).exists());
    let base_defaults = horizon_world::item_defaults::read(&base).unwrap();
    let defaults = horizon_world::item_defaults::read(&root).unwrap();
    assert_ne!(
        crate::content_identity::identity(&root, &defaults).unwrap(),
        crate::content_identity::identity(&base, &base_defaults).unwrap()
    );
    fs::remove_dir_all(&scratch).unwrap();
}

/// a living mod type's row in `entity-types.json` is its template's with `template`,
/// attribute bases and passenger attachments; other values keep the template's text.
/// a mod's thrown entity type gets the network id after vanilla's last, the
/// commands accept it, and it changes the content identity.
#[test]
#[ignore = "daily"]
fn content_root_registers_a_mod_entity_type() {
    let Some(base) = prepared() else {
        return;
    };
    let scratch = std::env::temp_dir().join(format!("horizon-mod-entity-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let m = folder_mod(
        &scratch.join("example"),
        &[
            ("horizon-mod.toml", &manifest("example", "")),
            (
                "data/example/horizon/item/gem.json",
                r#"{"template": "minecraft:snowball"}"#,
            ),
            (
                "data/example/horizon/entity_type/bolt.json",
                r#"{"template": "minecraft:snowball", "item": "example:gem", "damage": 2}"#,
            ),
        ],
    );
    let root = resource_root_fixture(
        &base,
        Path::new(""),
        std::slice::from_ref(&m),
        &scratch.join("cache"),
    )
    .unwrap();
    let report = |root: &Path| {
        horizon_protocol::registry::Builtin::load(&root.join("registries.json")).unwrap()
    };
    let (vanilla, ids) = (report(&base), report(&root));
    let vanilla = vanilla.get("minecraft:entity_type").unwrap();
    let ids = ids.get("minecraft:entity_type").unwrap();
    let count = (0..).take_while(|&i| vanilla.name(i).is_some()).count() as i32;
    assert_eq!(ids.id("example:bolt"), Some(count));
    assert_eq!(
        ids.id("minecraft:snowball"),
        vanilla.id("minecraft:snowball")
    );
    let commands: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("command-registries.json")).unwrap()).unwrap();
    for ids in [
        &commands["entity_types"],
        &commands["by_registry"]["minecraft:entity_type"]["ids"],
    ] {
        assert!(ids.as_array().unwrap().contains(&"example:bolt".into()));
    }
    // its size and eyes are the snowball's, for `execute anchored eyes` and spawners.
    let types: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("entity-types.json")).unwrap()).unwrap();
    assert!(types["minecraft:snowball"].is_object());
    let mut bolt = types["example:bolt"].clone();
    assert_eq!(bolt["template"], "minecraft:snowball");
    bolt.as_object_mut().unwrap().remove("template");
    assert_eq!(bolt, types["minecraft:snowball"]);
    // the rewritten table keeps vanilla's floats to the bit (a zombie's speed).
    let speed = |root: &Path| {
        let attributes =
            crate::mobs::exact_attributes(&fs::read(root.join("entity-types.json")).unwrap());
        attributes["minecraft:zombie"]["minecraft:movement_speed"]
    };
    assert_eq!(speed(&root).to_bits(), speed(&base).to_bits());
    assert!(root.join(crate::content_identity::MARKER).exists());
    let identity = crate::content_identity::identity;
    let defaults = horizon_world::item_defaults::read(&root).unwrap();
    let base_defaults = horizon_world::item_defaults::read(&base).unwrap();
    assert_ne!(
        identity(&root, &defaults).unwrap(),
        identity(&base, &base_defaults).unwrap()
    );
    fs::remove_dir_all(&scratch).unwrap();
}

/// a mod's menu type gets the menu network id after vanilla's last, and it
/// changes the content identity.
#[test]
#[ignore = "daily"]
fn content_root_registers_a_mod_menu_type() {
    let Some(base) = prepared() else {
        return;
    };
    let scratch = std::env::temp_dir().join(format!("horizon-mod-menu-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let m = folder_mod(
        &scratch.join("example"),
        &[
            ("horizon-mod.toml", &manifest("example", "")),
            (
                "data/example/horizon/menu/press.json",
                r#"{"slots": 9, "data_slots": 2, "results": [7, 8]}"#,
            ),
        ],
    );
    let root = resource_root_fixture(
        &base,
        Path::new(""),
        std::slice::from_ref(&m),
        &scratch.join("cache"),
    )
    .unwrap();
    let report = |root: &Path| {
        horizon_protocol::registry::Builtin::load(&root.join("registries.json")).unwrap()
    };
    let (vanilla, ids) = (report(&base), report(&root));
    let vanilla = vanilla.get("minecraft:menu").unwrap();
    let ids = ids.get("minecraft:menu").unwrap();
    let count = (0..).take_while(|&i| vanilla.name(i).is_some()).count() as i32;
    assert_eq!(ids.id("example:press"), Some(count));
    assert_eq!(ids.id("minecraft:furnace"), vanilla.id("minecraft:furnace"));
    assert!(root.join(crate::content_identity::MARKER).exists());
    let identity = crate::content_identity::identity;
    let defaults = horizon_world::item_defaults::read(&root).unwrap();
    let base_defaults = horizon_world::item_defaults::read(&base).unwrap();
    assert_ne!(
        identity(&root, &defaults).unwrap(),
        identity(&base, &base_defaults).unwrap()
    );
    fs::remove_dir_all(&scratch).unwrap();
}

/// A content mod folder from `(path, text)` pairs, read as the game reads it.
fn folder_mod(dir: &Path, files: &[(&str, &str)]) -> ContentMod {
    for (name, text) in files {
        let path = dir.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    let package = horizon_mod_package::Package::read(dir).unwrap();
    ContentMod {
        id: package.manifest.id.clone(),
        content: Content::read(dir, &package.manifest).unwrap(),
        jars: Vec::new(),
    }
}

/// The game mounts a flat package's entries as its checks read them: by the END record's
/// count. One it lists past that count, outside the mod's namespaces, is never loaded.
#[test]
fn a_package_entry_past_its_end_count_is_not_mounted() {
    use std::io::Write as _;
    let scratch = std::env::temp_dir().join(format!("horizon-mod-hidden-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    fs::create_dir_all(&scratch).unwrap();
    let path = scratch.join("ruby.hmod");
    let mut zip = zip::ZipWriter::new(fs::File::create(&path).unwrap());
    for (name, text) in [
        ("horizon-mod.toml", manifest("ruby", "")),
        ("assets/ruby/lang/en_us.json", "{}".to_owned()),
        ("data/minecraft/recipe/stone.json", "{}".to_owned()),
    ] {
        zip.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(text.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
    let ruby = || {
        let package = horizon_mod_package::Package::read(&path)?;
        Ok::<_, String>(ContentMod {
            id: "ruby".into(),
            content: Content::read(&path, &package.manifest)?,
            jars: Vec::new(),
        })
    };
    // As written, the fence refuses the vanilla recipe.
    assert!(ruby().is_err());
    // The END record now lists the first two entries only.
    let mut bytes = fs::read(&path).unwrap();
    let end = bytes.len() - 22;
    bytes[end + 8..end + 12].copy_from_slice(&[2, 0, 2, 0]);
    fs::write(&path, &bytes).unwrap();
    let mods = [ruby().unwrap()];
    assert!(load_layers(&scratch.join("jars"), &mods).is_err());
    let _ = fs::remove_dir_all(&scratch);
}

fn manifest(id: &str, namespaces: &str) -> String {
    format!(
        "id = \"{id}\"\nname = \"{id}\"\nversion = \"1.0\"\napi = \"0.2\"\ncapabilities = [\"registries\"]\nnamespaces = [{namespaces}]\n"
    )
}

/// A mod whose files sit under namespaces besides its id (Towns and Towers'
/// `kaisyn`): they join the root, and loaders read those namespaces.
#[test]
fn listed_namespaces_join_the_root() {
    let Some(base) = prepared() else {
        return;
    };
    let scratch = std::env::temp_dir().join(format!("horizon-mod-ns-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let toml = manifest("towers", "\"kaisyn\"");
    let m = folder_mod(
        &scratch.join("towers"),
        &[
            ("horizon-mod.toml", &toml),
            (
                "data/kaisyn/worldgen/processor_list/mossy.json",
                r#"{"processors": []}"#,
            ),
            (
                "data/kaisyn/tags/worldgen/biome/has_tower.json",
                r#"{"values": ["minecraft:plains"]}"#,
            ),
            (
                "assets/kaisyn/lang/en_us.json",
                r#"{"block.kaisyn.x": "X"}"#,
            ),
        ],
    );
    let root = scratch.join("root");
    build_fixture(&base, Path::new(""), &[m], &root).unwrap();
    assert_eq!(namespaces(&root), ["kaisyn", "towers"]);
    assert!(tag_namespaces(&root).contains(&"kaisyn".to_owned()));
    let lists: BTreeMap<String, String> =
        serde_json::from_slice(&fs::read(root.join("worldgen/processor_lists.json")).unwrap())
            .unwrap();
    assert!(lists.contains_key("kaisyn:mossy"));
    let tags: BTreeMap<String, String> =
        serde_json::from_slice(&fs::read(root.join("worldgen/biome_tags.json")).unwrap()).unwrap();
    assert!(tags.contains_key("kaisyn:has_tower"));
    let _ = fs::remove_dir_all(&scratch);
}

/// A mod's `assets/minecraft/atlases/blocks.json` adds its sources after vanilla's.
#[test]
fn a_content_root_appends_a_mods_atlas_sources() {
    let Some(base) = prepared() else {
        return;
    };
    let scratch = std::env::temp_dir().join(format!("horizon-mod-atlas-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let m = folder_mod(
        &scratch.join("gauge"),
        &[
            ("horizon-mod.toml", &manifest("gauge", "")),
            (
                "assets/minecraft/atlases/blocks.json",
                r#"{"sources": [{"type": "minecraft:directory", "source": "gauge_liquid", "prefix": "gauge_liquid/"}]}"#,
            ),
        ],
    );
    let root = scratch.join("root");
    build_fixture(&base, Path::new(""), std::slice::from_ref(&m), &root).unwrap();
    let resources = resolved_resources(
        &base,
        root.clone(),
        Path::new(""),
        std::slice::from_ref(&m),
        scratch.join("cache"),
    )
    .unwrap();
    let sources =
        |source: &horizon_assets::resource_source::ResourceSource| -> Vec<serde_json::Value> {
            let text = source.read("assets/minecraft/atlases/blocks.json").unwrap();
            let doc: serde_json::Value = serde_json::from_slice(&text).unwrap();
            doc["sources"].as_array().unwrap().clone()
        };
    let (vanilla, built) = (
        sources(&horizon_assets::resource_source::ResourceSource::directory(
            base.clone(),
        )),
        sources(resources.source()),
    );
    assert_eq!(built.len(), vanilla.len() + 1);
    assert_eq!(built[..vanilla.len()], vanilla[..]);
    assert_eq!(built[vanilla.len()]["source"], "gauge_liquid");
    let _ = fs::remove_dir_all(&scratch);
}

/// Two mods owning one namespace (one's listed namespace is the other's id,
/// or both list it) are refused together, naming both.
#[test]
fn mods_sharing_a_namespace_are_refused() {
    let scratch = std::env::temp_dir().join(format!("horizon-mod-clash-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let towers = folder_mod(
        &scratch.join("towers"),
        &[("horizon-mod.toml", &manifest("towers", "\"kaisyn\""))],
    );
    let kaisyn = folder_mod(
        &scratch.join("kaisyn"),
        &[("horizon-mod.toml", &manifest("kaisyn", ""))],
    );
    let other = folder_mod(
        &scratch.join("other"),
        &[("horizon-mod.toml", &manifest("other", "\"kaisyn\""))],
    );
    // The check runs before the base is read.
    let base = scratch.join("no-base");
    let out = scratch.join("root");
    let e = build_fixture(&base, Path::new(""), &[towers, kaisyn], &out).unwrap_err();
    assert_eq!(
        e.to_string(),
        "mods towers and kaisyn both use the namespace kaisyn; turn one of them off"
    );
    let towers = folder_mod(
        &scratch.join("towers"),
        &[("horizon-mod.toml", &manifest("towers", "\"kaisyn\""))],
    );
    let e = build_fixture(&base, Path::new(""), &[towers, other], &out).unwrap_err();
    assert!(e.to_string().starts_with("mods towers and other"), "{e}");
    let _ = fs::remove_dir_all(&scratch);
}

#[test]
#[ignore = "daily"]
fn conditional_tag_layers_replace_only_when_server_inventory_accepts_them() {
    let Some(base) = prepared() else { return };
    let scratch =
        std::env::temp_dir().join(format!("horizon-tag-conditions-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let file = "data/minecraft/tags/block/conditional_probe.json";
    let packages = [
        ("lower", r#"{"values":["minecraft:stone"]}"#),
        (
            "conditional",
            r#"{"neoforge:conditions":[{"type":"neoforge:mod_loaded","modid":"code_only"}],"replace":true,"values":["minecraft:dirt"]}"#,
        ),
        (
            "unsupported",
            r#"{"neoforge:conditions":[{"type":"unknown:condition"}],"replace":true,"values":["minecraft:diamond_block"]}"#,
        ),
        ("tail", r#"{"values":["minecraft:gold_block"]}"#),
    ];
    let mods: Vec<_> = packages
        .into_iter()
        .map(|(id, tag)| {
            folder_mod(
                &scratch.join(id),
                &[("horizon-mod.toml", &manifest(id, "")), (file, tag)],
            )
        })
        .collect();
    let inventory = |side: Option<mod_api::Side>| {
        horizon_mod_package::enabled::EnabledMods::new(
            mods.iter()
                .map(|m| (m.id.clone(), mod_api::Side::Both))
                .chain(side.map(|side| ("code_only".into(), side))),
        )
    };
    let cache = scratch.join("cache");
    let read = |root: &Path| -> serde_json::Value {
        serde_json::from_slice(&fs::read(root.join(file)).unwrap()).unwrap()
    };
    let off = super::resource_root(&base, Path::new(""), &mods, &cache, &inventory(None)).unwrap();
    assert_eq!(
        read(&off)["values"],
        serde_json::json!(["minecraft:diamond_block", "minecraft:gold_block"])
    );
    let on = super::resource_root(
        &base,
        Path::new(""),
        &mods,
        &cache,
        &inventory(Some(mod_api::Side::Both)),
    )
    .unwrap();
    assert_ne!(on, off, "code-only IDs invalidate the flattened tag cache");
    assert_eq!(
        read(&on)["values"],
        serde_json::json!(["minecraft:diamond_block", "minecraft:gold_block"])
    );
    assert!(read(&on).get("neoforge:conditions").is_none());
    let client = super::resource_root(
        &base,
        Path::new(""),
        &mods,
        &cache,
        &inventory(Some(mod_api::Side::Client)),
    )
    .unwrap();
    assert_eq!(read(&client)["values"], read(&off)["values"]);
    let again =
        super::resource_root(&base, Path::new(""), &mods, &cache, &inventory(None)).unwrap();
    assert_eq!(again, off);
    assert_eq!(
        read(&off)["values"],
        serde_json::json!(["minecraft:diamond_block", "minecraft:gold_block"])
    );
    fs::remove_dir_all(scratch).unwrap();
}

#[path = "mod_content_tests_tests.rs"]
mod more;

/// A replacing mod's tag leaves vanilla's entries out of the tag file and of
/// the worldgen loader's biome tags, and the root keeps the warning for the
/// starts that reuse it.
#[test]
fn a_replacing_mod_tag_reaches_the_root() {
    let Some(base) = prepared() else {
        return;
    };
    let scratch = std::env::temp_dir().join(format!("horizon-mod-replace-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let file = "data/minecraft/tags/worldgen/biome/has_structure/village_plains.json";
    let adds = folder_mod(
        &scratch.join("adds"),
        &[
            ("horizon-mod.toml", &manifest("adds", "")),
            (file, r#"{"values": ["minecraft:forest"]}"#),
        ],
    );
    let towns = folder_mod(
        &scratch.join("towns"),
        &[
            ("horizon-mod.toml", &manifest("towns", "")),
            (file, r#"{"replace": true, "values": ["minecraft:plains"]}"#),
        ],
    );
    let late = folder_mod(
        &scratch.join("late"),
        &[
            ("horizon-mod.toml", &manifest("late", "")),
            (file, r#"{"values": ["minecraft:savanna"]}"#),
        ],
    );
    let vanilla: serde_json::Value =
        serde_json::from_slice(&fs::read(base.join(file)).unwrap()).unwrap();
    assert!(vanilla["values"].as_array().unwrap().len() > 1, "{vanilla}");
    let root = resource_root_fixture(
        &base,
        Path::new(""),
        &[adds, towns, late],
        &scratch.join("cache"),
    )
    .unwrap();
    let expected = serde_json::json!(["minecraft:plains", "minecraft:savanna"]);
    let tag: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join(file)).unwrap()).unwrap();
    assert_eq!(tag["values"], expected);
    let tags: BTreeMap<String, String> =
        serde_json::from_slice(&fs::read(root.join("worldgen/biome_tags.json")).unwrap()).unwrap();
    let bundled: serde_json::Value =
        serde_json::from_str(&tags["minecraft:has_structure/village_plains"]).unwrap();
    assert_eq!(bundled["values"], expected);
    let warnings = fs::read_to_string(root.join(WARNINGS)).unwrap();
    assert!(
        warnings.starts_with("mod towns replaces data/minecraft/tags/worldgen/biome/has_structure/village_plains.json: vanilla's entries and those of adds are left out"),
        "{warnings}"
    );
    let _ = fs::remove_dir_all(&scratch);
}

/// An old chunk's saved jigsaw start comes back with its pieces (`StructureStart.loadStaticStart`):
/// a village this seed makes, saved as an older version would have, is loaded from its `Children`
/// and has the same pieces, boxes and positions, so the neighbours can place what crosses into
/// them. A legacy structure with no loader is the generator's own start when it has the same
/// pieces' box, and name and box alone when it has not.
#[test]
#[ignore = "daily"]
fn an_old_chunks_saved_village_comes_back_with_its_pieces() {
    use horizon_core::position::ChunkPos;
    use {
        horizon_core::nbt::Tag,
        horizon_world::worldgen::blending::{OldWorld, SavedStart},
    };
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
    let open = || crate::vanillagen::VanillaTerrain::new(terrain_resources, 1234).unwrap();
    let villages: Vec<String> = ["plains", "desert", "savanna", "snowy", "taiga"]
        .iter()
        .map(|k| format!("minecraft:village_{k}"))
        .collect();
    let made = open();
    let (name, at) = made
        .locate(&villages, [0, 64, 0], 100)
        .expect("a village near the origin");
    let (cx, cz) = (at[0] >> 4, at[2] >> 4);
    let original = made
        .starts_at(cx, cz)
        .into_iter()
        .find(|s| s.name == name)
        .expect("the village start");
    assert!(original.pieces.len() > 1);
    // Saved the way a chunk keeps it: the start's tag, its Children and their union box.
    let Some(Tag::Compound(saved)) = original.save() else {
        panic!("the village saves");
    };
    let Some(Tag::List(children)) = saved.get(&"Children".into()).cloned() else {
        panic!("Children");
    };
    let bounds = original
        .pieces
        .iter()
        .map(|p| p.bounding_box())
        .fold(None::<[i32; 6]>, |u, b| {
            let [x0, y0, z0, x1, y1, z1] =
                [b.min[0], b.min[1], b.min[2], b.max[0], b.max[1], b.max[2]];
            Some(match u {
                None => [x0, y0, z0, x1, y1, z1],
                Some(u) => [
                    u[0].min(x0),
                    u[1].min(y0),
                    u[2].min(z0),
                    u[3].max(x1),
                    u[4].max(y1),
                    u[5].max(z1),
                ],
            })
        });
    struct Saved {
        chunk: (i32, i32),
        start: SavedStart,
    }
    impl OldWorld for Saved {
        fn is_old_chunk_around(&self, center: ChunkPos, range: i32) -> bool {
            (center.x - self.chunk.0).abs() <= range && (center.z - self.chunk.1).abs() <= range
        }
        fn blending(
            &self,
            _: i32,
            _: i32,
        ) -> Option<std::sync::Arc<horizon_world::worldgen::blending::BlendingData>> {
            None
        }
        fn old_biomes(
            &self,
            _: i32,
            _: i32,
        ) -> Option<
            std::sync::Arc<
                horizon_world::worldgen::biome_chunk::BiomeChunk<
                    horizon_world::worldgen::biome::BiomeId,
                >,
            >,
        > {
            None
        }
        fn saved_starts(&self, x: i32, z: i32) -> Vec<SavedStart> {
            if (x, z) == self.chunk {
                vec![self.start.clone()]
            } else {
                Vec::new()
            }
        }
    }
    let loaded = open();
    loaded.set_old_world(std::sync::Arc::new(Saved {
        chunk: (cx, cz),
        // No box to compare: the pieces can only come from the Children.
        start: SavedStart {
            name: name.clone(),
            bounds: None,
            pieces: children.clone(),
        },
    }));
    let back = loaded
        .starts_at(cx, cz)
        .into_iter()
        .find(|s| s.name == name)
        .expect("the saved start");
    assert_eq!(
        back.pieces.len(),
        original.pieces.len(),
        "every piece is loaded"
    );
    for (a, b) in original.pieces.iter().zip(&back.pieces) {
        assert_eq!(a.bounding_box(), b.bounding_box());
        assert_eq!(a.type_name(), b.type_name());
        assert_eq!(a.save(), b.save(), "a piece saves as it was loaded");
    }
    assert_eq!(back.bounding_box, original.bounding_box);
    // No pieces to load and a box no generated start has: the name and the box only.
    let odd = open();
    odd.set_old_world(std::sync::Arc::new(Saved {
        chunk: (cx, cz),
        start: SavedStart {
            name: name.clone(),
            bounds: Some([0, 0, 0, 1, 1, 1]),
            pieces: Vec::new(),
        },
    }));
    let stub = odd
        .starts_at(cx, cz)
        .into_iter()
        .find(|s| s.name == name)
        .unwrap();
    assert!(stub.pieces.is_empty());
    assert_eq!(stub.bounding_box.min, [0, 0, 0]);
    // The box of the start this generator makes there: it is that start.
    let same = open();
    same.set_old_world(std::sync::Arc::new(Saved {
        chunk: (cx, cz),
        start: SavedStart {
            name: name.clone(),
            bounds,
            pieces: Vec::new(),
        },
    }));
    let rebuilt = same
        .starts_at(cx, cz)
        .into_iter()
        .find(|s| s.name == name)
        .unwrap();
    assert_eq!(rebuilt.pieces.len(), original.pieces.len());
}

/// A recipe's `result` is a stack's fields or a bare item id, as vanilla's `ItemStackTemplate`
/// codec reads it (#4904): a mod's recipes make its own items either way.
#[test]
#[ignore = "daily"]
fn mod_recipe_results_are_an_item_stack_or_a_bare_item_id() {
    let Some(base) = prepared() else {
        return;
    };
    let scratch = std::env::temp_dir().join(format!("horizon-mod-results-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let dir = scratch.join("ruby");
    let _ = ruby_mod(&dir);
    // Each recipe takes one item no vanilla recipe crafts alone and makes one of the mod's.
    let cases = [
        (
            "stack",
            "minecraft:diamond",
            r#"{"id": "ruby:ruby"}"#,
            "ruby:ruby",
            1,
        ),
        (
            "bare_id",
            "minecraft:flint",
            r#""ruby:ruby""#,
            "ruby:ruby",
            1,
        ),
        (
            "bare_block_item",
            "minecraft:ghast_tear",
            r#""ruby:ruby_block""#,
            "ruby:ruby_block",
            1,
        ),
        (
            "counted",
            "minecraft:ender_pearl",
            r#"{"id": "ruby:ruby_block", "count": 3}"#,
            "ruby:ruby_block",
            3,
        ),
    ];
    for (name, input, result, _, _) in cases {
        let recipe = format!(
            r#"{{"type": "minecraft:crafting_shapeless", "ingredients": ["{input}"], "result": {result}}}"#
        );
        fs::write(dir.join(format!("data/ruby/recipe/{name}.json")), recipe).unwrap();
    }
    let package = horizon_mod_package::Package::read(&dir).unwrap();
    let m = ContentMod {
        id: "ruby".into(),
        content: Content::read(&dir, &package.manifest).unwrap(),
        jars: Vec::new(),
    };
    // Readers of the root hold its lock; Windows removes no folder with an open file.
    {
        let relative = relative_base(&base);
        let root = resource_root_fixture(
            &relative,
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
        let files = content_files(&resources);
        let overlay = PackOverlay::new(&files);
        let assets = Assets::load(&root).unwrap();
        let new = assets.registry;
        let facts = BlockFacts::load(&root, new).unwrap();
        let defaults = horizon_world::item_defaults::read(&root).unwrap();
        let items = Items::load(&root, new, &facts, &overlay, &defaults).unwrap();
        let recipes = crate::crafting::Recipes::load(
            &root,
            &items,
            &overlay,
            &crate::test_resources::server_load(),
        )
        .unwrap();
        for (name, input, _, item, count) in cases {
            let grid = [
                Some(crate::items::ItemStack::new(input, 1)),
                None,
                None,
                None,
            ];
            let (_, made) = recipes
                .find(crate::test_resources::server_loop_context(), &grid, 2)
                .unwrap_or_else(|| panic!("{name}: no recipe"));
            assert_eq!((&*made.item, made.count), (item, count), "{name}");
        }
    }
    fs::remove_dir_all(&scratch).unwrap();
}

/// A mod's creative tab comes after vanilla's, on page 1 in the first top
/// slot, with only the items the game has; its new items join the search tab
/// under their English names.
#[test]
#[ignore = "daily"]
fn creative_tabs_come_on_their_own_page() {
    let Some(base) = prepared() else {
        return;
    };
    let scratch = std::env::temp_dir().join(format!("horizon-mod-tabs-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let dir = scratch.join("gems");
    for (name, text) in [
        (
            "horizon-mod.toml",
            "id = \"gems\"\nname = \"Gems\"\nversion = \"0.1.0\"\napi = \"0.2\"\nside = \"both\"\ncapabilities = [\"registries\"]\n",
        ),
        (
            "data/gems/horizon/item/ruby.json",
            r#"{"template": "minecraft:diamond"}"#,
        ),
        (
            "data/gems/horizon/block/ruby_block.json",
            r#"{"template": "minecraft:diamond_block"}"#,
        ),
        (
            "data/gems/horizon/creative_tab/gems.json",
            r#"{"title": "Gems", "icon": "gems:ruby", "items": ["gems:ruby", "minecraft:diamond", "gems:missing"]}"#,
        ),
        (
            "assets/gems/lang/en_us.json",
            r#"{"item.gems.ruby": "Ruby"}"#,
        ),
    ] {
        let path = dir.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    let package = horizon_mod_package::Package::read(&dir).unwrap();
    let m = ContentMod {
        id: "gems".into(),
        content: Content::read(&dir, &package.manifest).unwrap(),
        jars: Vec::new(),
    };
    let relative = relative_base(&base);
    let root = resource_root_fixture(
        &relative,
        Path::new(""),
        std::slice::from_ref(&m),
        &scratch.join("cache"),
    )
    .unwrap();
    let mut creative = crate::creative::Creative::load(&root).unwrap();
    assert_eq!(creative.pages(), 2);
    let i = creative
        .tabs
        .iter()
        .position(|t| t.id == "gems:gems")
        .unwrap();
    let tab = &creative.tabs[i];
    assert_eq!(
        (tab.page, tab.top, tab.column, tab.title.as_str()),
        (1, true, 0, "Gems")
    );
    let items: Vec<&str> = tab.items.iter().map(|s| &*s.item).collect();
    assert_eq!(items, ["gems:ruby", "minecraft:diamond"]);
    assert!(
        creative
            .search_items()
            .iter()
            .any(|s| &*s.item == "gems:ruby")
    );
    // A block's item is in search once.
    let blocks = creative
        .search_items()
        .iter()
        .filter(|s| &*s.item == "gems:ruby_block");
    assert_eq!(blocks.count(), 1);
    // Search, on every page, stays selected over a page turn.
    let search = creative.tabs.iter().position(|t| t.search).unwrap();
    creative.select(search);
    creative.set_page(1);
    assert_eq!((creative.selected, creative.page), (search, 1));
    creative.select(0);
    // Page 1 shows the mod's tab and the right-hand ones, not vanilla's.
    creative.set_page(1);
    assert_eq!(creative.selected, i);
    let shown: Vec<&str> = creative
        .tabs
        .iter()
        .filter(|t| creative.on_page(t))
        .map(|t| t.id.as_str())
        .collect();
    assert!(
        shown.contains(&"gems:gems") && shown.contains(&"minecraft:search"),
        "{shown:?}"
    );
    assert!(!shown.contains(&"minecraft:building_blocks"), "{shown:?}");
    // Selecting a vanilla tab goes back to its page.
    creative.select(0);
    assert_eq!(creative.page, 0);
    fs::remove_dir_all(&scratch).unwrap();
}

/// A block that gives each of its template's states a run moves a template
/// state to the first of its own: the combination every declared property is
/// at its default for, which is the one the id cursor points at first.
#[test]
fn a_fan_out_rename_moves_a_template_state_to_its_first_state() {
    let mut r = Rename::new("minecraft:test", "testmod:test");
    r.states = 10..12;
    r.first = 100;
    r.fanout = 6;
    assert_eq!(
        (r.id(9), r.id(10), r.id(11), r.id(12)),
        (None, Some(100), Some(106), None)
    );
}

/// A base root of one template with two states, and a mod that adds a block
/// to it. `declarations` go in as the block file.
fn block_base(tag: &str, declarations: &[&str]) -> (PathBuf, Vec<ContentMod>) {
    block_base_of(
        tag,
        declarations,
        r#"{"states":[
      ["minecraft:test",false,false,"",{"id":"minecraft:test","properties":{"axis":"x"}}],
      ["minecraft:test",false,false,"",{"id":"minecraft:test","properties":{"axis":"y"}}]],
      "defaults":{"minecraft:test":0}}"#,
    )
}

/// The same over a base root whose `facts.json` is `facts`: the other tables
/// the plan reads are built from its state count, so a template can have one
/// state and no properties of its own (a `stone`) as easily as two.
fn block_base_of(tag: &str, declarations: &[&str], facts: &str) -> (PathBuf, Vec<ContentMod>) {
    let scratch = std::env::temp_dir().join(format!("horizon-mod-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let base = scratch.join("base");
    fs::create_dir_all(&base).unwrap();
    fs::write(base.join("facts.json"), facts).unwrap();
    let count = match serde_json::from_str::<J>(facts).unwrap().get("states") {
        Some(J::Arr(rows)) => rows.len(),
        _ => panic!("{tag}: facts.json has no states"),
    };
    let rows = vec![r#"{"item":"minecraft:test"}"#; count].join(",");
    fs::write(
        base.join("gameplay-facts.json"),
        format!(r#"{{"states":[{rows}]}}"#),
    )
    .unwrap();
    fs::write(base.join("items.json"), r#"{"minecraft:test":{}}"#).unwrap();
    fs::write(
        base.join("registries.json"),
        r#"{"minecraft:item":{"entries":{"minecraft:test":{"protocol_id":0}}},"minecraft:block":{"entries":{"minecraft:test":{"protocol_id":0}}}}"#,
    )
    .unwrap();
    let mods: Vec<ContentMod> = declarations
        .iter()
        .enumerate()
        .map(|(i, declaration)| {
            let dir = scratch.join(format!("mod{i}"));
            folder_mod(
                &dir,
                &[
                    ("horizon-mod.toml", &manifest("testmod", "")),
                    ("data/testmod/horizon/block/test.json", *declaration),
                ],
            )
        })
        .collect();
    (base, mods)
}

/// An NBT state row for the two-state `minecraft:test` template, the shape
/// `BlockState::to_nbt` writes.
fn test_state_row(axis: &str) -> Tag {
    Tag::Compound(Compound::from([
        (
            JavaString::from("id"),
            Tag::String(JavaString::from("minecraft:test")),
        ),
        (
            JavaString::from("properties"),
            Tag::Compound(Compound::from([(
                JavaString::from("axis"),
                Tag::String(JavaString::from(axis)),
            )])),
        ),
    ]))
}

/// A base root of one block with three biomes, two biome tags (one nesting
/// the other), three vanilla placed features and entity types: what biome
/// modifiers read.
fn biome_base(tag: &str) -> PathBuf {
    let (base, _) = block_base(tag, &[]);
    let worldgen = base.join("worldgen");
    fs::create_dir_all(&worldgen).unwrap();
    let bundle = |entries: &[(&str, &str)]| -> String {
        let map: BTreeMap<&str, &str> = entries.iter().copied().collect();
        serde_json::to_string(&map).unwrap()
    };
    let files = [
        (
            "biomes.json",
            bundle(&[
                (
                    "minecraft:plains",
                    r#"{"temperature":0.8,"downfall":0.4,"features":[[],["minecraft:lake"],[],[],[],[],["minecraft:ore_coal"]],"attributes":{"minecraft:gameplay/natural_mob_spawns":{"argument":{"spawn_costs":{"minecraft:skeleton":{"energy_budget":0.7,"charge":0.15}},"spawns_by_category":{"monster":[{"type":"minecraft:zombie","count":4,"weight":95},{"type":"minecraft:skeleton","count":4,"weight":100}]}},"modifier":"overlay"}}}"#,
                ),
                (
                    "minecraft:desert",
                    r#"{"temperature":2.0,"downfall":0.0,"features":[["minecraft:fossil"]]}"#,
                ),
                (
                    "minecraft:nether_wastes",
                    r#"{"temperature":2.0,"downfall":0.0,"features":[[],[],[],[],[],[],["minecraft:ore_coal"]]}"#,
                ),
            ]),
        ),
        (
            "biome_tags.json",
            bundle(&[
                (
                    "minecraft:is_overworld",
                    r##"{"values":["#minecraft:is_plains_like","minecraft:desert",{"id":"othermod:x","required":false}]}"##,
                ),
                (
                    "minecraft:is_plains_like",
                    r#"{"values":["minecraft:plains"]}"#,
                ),
            ]),
        ),
        (
            "placed_features.json",
            bundle(&[
                ("minecraft:ore_coal", "{}"),
                ("minecraft:lake", "{}"),
                ("minecraft:fossil", "{}"),
            ]),
        ),
        ("features.json", bundle(&[])),
    ];
    for (name, text) in files {
        fs::write(worldgen.join(name), text).unwrap();
    }
    fs::write(
        base.join("entity-types.json"),
        r#"{"minecraft:zombie":{"category":"monster"},"minecraft:skeleton":{"category":"monster"},"minecraft:stray":{"category":"monster"},"minecraft:rabbit":{"category":"creature"}}"#,
    )
    .unwrap();
    let entity_tags = base.join("data/minecraft/tags/entity_type");
    fs::create_dir_all(&entity_tags).unwrap();
    fs::write(
        entity_tags.join("skeletons.json"),
        r#"{"values":["minecraft:skeleton","minecraft:stray",{"id":"othermod:bone","required":false}]}"#,
    )
    .unwrap();
    base
}

/// A mod adding a zinc ore and its biome modifiers (`(file name, text)`).
fn zinc_mod(dir: &Path, modifiers: &[(&str, &str)]) -> ContentMod {
    let mut files = vec![
        ("horizon-mod.toml".to_owned(), manifest("zinc", "")),
        (
            "data/zinc/worldgen/feature/ore_zinc.json".to_owned(),
            r#"{"type":"minecraft:ore","config":{}}"#.to_owned(),
        ),
        (
            "data/zinc/worldgen/placed_feature/ore_zinc.json".to_owned(),
            r#"{"feature":"zinc:ore_zinc","placement":[]}"#.to_owned(),
        ),
    ];
    for (name, text) in modifiers {
        files.push((
            format!("data/zinc/neoforge/biome_modifier/{name}.json"),
            (*text).to_owned(),
        ));
    }
    let files: Vec<(&str, &str)> = files
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    folder_mod(dir, &files)
}

fn biome_features(root: &Path) -> BTreeMap<String, serde_json::Value> {
    let bundle: BTreeMap<String, String> =
        serde_json::from_slice(&fs::read(root.join("worldgen/biomes.json")).unwrap()).unwrap();
    bundle
        .into_iter()
        .map(|(id, text)| {
            let v: serde_json::Value = serde_json::from_str(&text).unwrap();
            (id, v["features"].clone())
        })
        .collect()
}

/// Blocks and items a native mod declares in code join the content plan:
/// inside the content mod of their namespace, or as one of their own, with
/// ids after vanilla's as files' are.
#[test]
#[ignore = "daily"]
fn blocks_and_items_declared_in_code_get_ids() {
    use horizon_mod_package::content::{Block, Item};
    let Some(base) = prepared() else {
        return;
    };
    let scratch = std::env::temp_dir().join(format!("horizon-mod-declared-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let known = known_components(&base).unwrap();
    let block = |id: &str| Block {
        id: id.into(),
        template: "minecraft:oak_log".into(),
        item: true,
        ..Block::default()
    };
    let wrench = Item {
        id: "gears:wrench".into(),
        template: "minecraft:stick".into(),
        max_stack_size: Some(1),
        ..Item::default()
    };
    let mut mods = vec![ruby_mod(&scratch.join("ruby"))];
    let errors = accept_declared(
        &mut mods,
        &[block("ruby:ruby_log"), block("gears:shaft")],
        &[wrench],
        &[],
        &known,
    );
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(mods.len(), 2);
    assert!(
        mods[0]
            .content
            .blocks
            .iter()
            .any(|b| b.id == "ruby:ruby_log")
    );
    assert_eq!(mods[1].id, "gears");
    // A namespace another mod owns, and a clash with a file's block, are refused whole.
    let refused = accept_declared(
        &mut mods,
        &[block("ruby:ruby_block"), block("ruby:ruby_post")],
        &[],
        &[],
        &known,
    );
    assert_eq!(refused.len(), 1, "{refused:?}");
    assert!(
        !mods[0]
            .content
            .blocks
            .iter()
            .any(|b| b.id == "ruby:ruby_post")
    );

    let root = resource_root_fixture(&base, Path::new(""), &mods, &scratch.join("cache")).unwrap();
    let vanilla = Assets::load(&base).unwrap();
    let assets = Assets::load(&root).unwrap();
    let n = vanilla.registry.len() as u32;
    let log_states = (0..n)
        .filter(|&s| vanilla.registry.state(BlockStateId(s)).unwrap().name == "minecraft:oak_log")
        .count() as u32;
    // Sorted by mod, then block: gears:shaft, then ruby's ruby_block, ruby_door, ruby_glass, ruby_log.
    let shaft = (n..assets.registry.len() as u32)
        .find(|&s| assets.registry.state(BlockStateId(s)).unwrap().name == "gears:shaft")
        .unwrap();
    assert_eq!(shaft, n);
    assert!(assets.registry.default_state("ruby:ruby_log").is_some());
    assert_eq!(
        assets.registry.len() as u32,
        n + log_states + 1 + 64 + 1 + log_states
    );
    let ids = horizon_protocol::registry::Builtin::load(&root.join("registries.json")).unwrap();
    for item in ["gears:shaft", "gears:wrench", "ruby:ruby_log"] {
        assert!(
            ids.get("minecraft:item").unwrap().id(item).is_some(),
            "{item}"
        );
    }
    let facts = BlockFacts::load(&root, assets.registry).unwrap();
    let defaults = horizon_world::item_defaults::read(&root).unwrap();
    let items = Items::load(
        &root,
        assets.registry,
        &facts,
        &PackOverlay::NONE,
        &defaults,
    )
    .unwrap();
    assert_eq!(items.items["gears:wrench"].max_stack, 1);
    assert!(items.items["gears:shaft"].block.is_some());
    // The same declarations reuse the root; others build a new one.
    assert_eq!(
        resource_root_fixture(&base, Path::new(""), &mods, &scratch.join("cache")).unwrap(),
        root
    );
    // Changed declarations build a root of their own: the old one stays as
    // it was, for a switch back to that set.
    let stamp = fs::read(root.join(STAMP)).unwrap();
    mods[1].content.items[0].max_stack_size = Some(2);
    let changed =
        resource_root_fixture(&base, Path::new(""), &mods, &scratch.join("cache")).unwrap();
    assert_ne!(changed, root);
    assert_eq!(fs::read(root.join(STAMP)).unwrap(), stamp);
    assert_ne!(fs::read(changed.join(STAMP)).unwrap(), stamp);
    let changed_defaults = horizon_world::item_defaults::read(&changed).unwrap();
    let changed_items = Items::load(
        &changed,
        assets.registry,
        &facts,
        &PackOverlay::NONE,
        &changed_defaults,
    )
    .unwrap();
    assert_eq!(changed_items.items["gears:wrench"].max_stack, 2);
    assert_eq!(items.items["gears:wrench"].max_stack, 1);
    let _ = fs::remove_dir_all(&scratch);
}

#[test]
fn a_file_item_whose_remainder_was_refused_takes_only_its_mod_out() {
    use horizon_mod_package::content::Item;
    let item = |id: &str, remainder: Option<&str>| Item {
        id: id.into(),
        template: "minecraft:diamond".into(),
        crafting_remainder: remainder.map(str::to_owned),
        ..Item::default()
    };
    let make = |id: &str, items: Vec<Item>| {
        let mut content = Content::declared(id);
        content.items = items;
        ContentMod {
            id: id.into(),
            content,
            jars: Vec::new(),
        }
    };
    let mut jar = make("jar", vec![item("jar:tool", Some("jar:jar"))]);
    jar.content
        .blocks
        .push(horizon_mod_package::content::Block {
            id: "jar:jar".into(),
            template: "minecraft:glass".into(),
            item: true,
            ..Default::default()
        });
    let mut mods = vec![
        jar,
        make(
            "good",
            vec![
                item("good:tool", Some("good:wrench")),
                item("good:wrench", None),
            ],
        ),
        make("bad", vec![item("bad:tool", Some("bad:wrench"))]),
        make(
            "vanilla",
            vec![item("vanilla:soup", Some("minecraft:bowl"))],
        ),
    ];
    let dropped = drop_dangling_remainders(&mut mods);
    assert_eq!(dropped.len(), 1, "{dropped:?}");
    assert!(dropped[0].contains("bad"), "{dropped:?}");
    let left: Vec<&str> = mods.iter().map(|m| m.id.as_str()).collect();
    // A remainder that is the mod's own block item is kept too (`jar`).
    assert_eq!(left, ["jar", "good", "vanilla"]);
}

/// A sign's four forms come from its template, whatever the blocks are named,
/// its wood from its declaration, and its item places the wall form it names
/// and is that form's item too. A `road_sign` of stone is no sign.
#[test]
#[ignore = "daily"]
fn signs_take_their_form_wood_and_wall_block_from_their_declaration() {
    use horizon_world::level::{Behaviours, ItemKind};

    let Some(base) = prepared() else {
        return;
    };
    let scratch = std::env::temp_dir().join(format!("horizon-mod-signs-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let m = folder_mod(
        &scratch.join("signs"),
        &[
            ("horizon-mod.toml", &manifest("signs", "")),
            (
                "data/signs/horizon/block/notice.json",
                r#"{"template": "minecraft:oak_sign", "wood_type": "signs:elm", "wall_block": "signs:board"}"#,
            ),
            (
                "data/signs/horizon/block/board.json",
                r#"{"template": "minecraft:oak_wall_sign", "wood_type": "signs:elm", "item": false}"#,
            ),
            (
                "data/signs/horizon/block/lantern.json",
                r#"{"template": "minecraft:spruce_hanging_sign", "wall_block": "signs:bracket"}"#,
            ),
            (
                "data/signs/horizon/block/bracket.json",
                r#"{"template": "minecraft:spruce_wall_hanging_sign", "wood_type": "cherry", "item": false}"#,
            ),
            (
                "data/signs/horizon/block/road_sign.json",
                r#"{"template": "minecraft:stone"}"#,
            ),
        ],
    );
    let root = resource_root_fixture(
        &base,
        Path::new(""),
        std::slice::from_ref(&m),
        &scratch.join("cache"),
    )
    .unwrap();
    let assets = Assets::load(&root).unwrap();
    let registry = assets.registry;
    let facts = BlockFacts::load(&root, registry).unwrap();
    let behaviours = Behaviours::load(registry, &root).unwrap();
    let block = |name: &str| facts.get(registry.default_state(name).unwrap());
    let form = |hanging, wall| Some(crate::blocks::SignForm { hanging, wall });
    assert_eq!(block("signs:notice").sign_form(), form(false, false));
    assert_eq!(block("signs:board").sign_form(), form(false, true));
    assert_eq!(block("signs:lantern").sign_form(), form(true, false));
    assert_eq!(block("signs:bracket").sign_form(), form(true, true));
    assert_eq!(block("signs:road_sign").sign_form(), None);
    assert_eq!(
        block("minecraft:oak_wall_sign").sign_form(),
        form(false, true)
    );
    // The declared wood, or the template's.
    assert_eq!(facts.wood_type("signs:notice"), Some("signs:elm"));
    assert_eq!(facts.wood_type("signs:bracket"), Some("cherry"));
    assert_eq!(facts.wood_type("signs:lantern"), Some("spruce"));
    assert_eq!(
        facts.wood_type("minecraft:dark_oak_wall_sign"),
        Some("dark_oak")
    );
    assert_eq!(facts.wood_type("signs:road_sign"), None);
    // Each item places its sign and the wall form it names, which it is the item of.
    let id = |name: &str| behaviours.table.block_by_name(name);
    let notice = behaviours.item_binding("signs:notice").unwrap();
    assert_eq!(notice.kind, ItemKind::StandingAndWall);
    assert_eq!(
        (Some(notice.block), notice.wall_block),
        (id("signs:notice"), id("signs:board"))
    );
    let lantern = behaviours.item_binding("signs:lantern").unwrap();
    assert_eq!(lantern.kind, ItemKind::HangingSign);
    assert_eq!(
        (Some(lantern.block), lantern.wall_block),
        (id("signs:lantern"), id("signs:bracket"))
    );
    assert_eq!(&*block("signs:board").item, "signs:notice");
    assert_eq!(&*block("signs:bracket").item, "signs:lantern");
    let defaults = horizon_world::item_defaults::read(&root).unwrap();
    let mut renderer = crate::block_entity_render::BlockEntityRenderer::load(
        &horizon_assets::resource_source::ResourceRoot::directory(root.to_path_buf()),
        registry,
        &defaults,
    )
    .unwrap();
    renderer.add_mod_signs(&facts, registry);
    for name in [
        "signs:notice",
        "signs:board",
        "signs:lantern",
        "signs:bracket",
    ] {
        assert!(
            renderer.renders(registry.default_state(name).unwrap().0),
            "{name}"
        );
    }
    assert!(!renderer.renders(registry.default_state("signs:road_sign").unwrap().0));
    // Vanilla's pair stays as it was.
    let oak = behaviours.item_binding("minecraft:oak_sign").unwrap();
    assert_eq!(oak.wall_block, id("minecraft:oak_wall_sign"));
    let _ = fs::remove_dir_all(&scratch);
}

/// A fence gate of a mod is a vanilla `FenceGateBlock` through its template, with
/// its wood's sounds unless the file names its own: `open_sound` and
/// `close_sound` replace `WoodType.fenceGateOpen` and `fenceGateClose`, the one
/// value a use, a redstone change and an explosion all read.
#[test]
#[ignore = "daily"]
fn a_content_fence_gate_plays_its_own_sounds_or_its_woods() {
    use horizon_world::level::Behaviours;

    let Some(base) = prepared() else {
        return;
    };
    let scratch = std::env::temp_dir().join(format!("horizon-mod-gates-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let m = folder_mod(
        &scratch.join("gates"),
        &[
            ("horizon-mod.toml", &manifest("gates", "")),
            (
                "data/gates/horizon/block/oak.json",
                r#"{"template": "minecraft:oak_fence_gate"}"#,
            ),
            (
                "data/gates/horizon/block/rope.json",
                r#"{"template": "minecraft:oak_fence_gate", "open_sound": "gates:rope.open", "close_sound": "minecraft:block.chain.break"}"#,
            ),
            (
                "data/gates/horizon/block/half.json",
                r#"{"template": "minecraft:birch_fence_gate", "open_sound": "gates:half.open"}"#,
            ),
        ],
    );
    let root = resource_root_fixture(
        &base,
        Path::new(""),
        std::slice::from_ref(&m),
        &scratch.join("cache"),
    )
    .unwrap();
    let registry = Assets::load(&root).unwrap().registry;
    let behaviours = Behaviours::load(registry, &root).unwrap();
    let sounds = |name: &str| {
        let state = registry.default_state(name).unwrap();
        let data = behaviours.data(state);
        (
            data.gate_open.as_deref().map(str::to_owned),
            data.gate_close.as_deref().map(str::to_owned),
        )
    };
    let gate = |open: Option<&str>, close: Option<&str>| {
        (open.map(str::to_owned), close.map(str::to_owned))
    };
    assert_eq!(
        sounds("gates:oak"),
        gate(
            Some("minecraft:block.fence_gate.open"),
            Some("minecraft:block.fence_gate.close")
        )
    );
    assert_eq!(
        sounds("gates:rope"),
        gate(Some("gates:rope.open"), Some("minecraft:block.chain.break"))
    );
    assert_eq!(
        sounds("gates:half"),
        gate(
            Some("gates:half.open"),
            Some("minecraft:block.fence_gate.close")
        )
    );
    assert_eq!(
        sounds("minecraft:oak_fence_gate"),
        sounds("gates:oak"),
        "the vanilla gate keeps its sounds"
    );
    let _ = fs::remove_dir_all(&scratch);
}

/// A block's item is one value in its gameplay row: its own id by default, the
/// `block_item` it names (a mod's own item or a vanilla one), or none with
/// `item` false. A `block_item` that is no item refuses the load. Pick-block's item
/// is that value unless the block declares a `pick_item` of its own.
#[test]
#[ignore = "daily"]
fn a_block_yields_the_item_it_names_or_its_own_or_none() {
    use horizon_world::level::Behaviours;

    let Some(base) = prepared() else {
        return;
    };
    let scratch =
        std::env::temp_dir().join(format!("horizon-mod-blockitem-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let m = folder_mod(
        &scratch.join("pick"),
        &[
            ("horizon-mod.toml", &manifest("pick", "")),
            (
                "data/pick/horizon/block/plain.json",
                r#"{"template": "minecraft:stone"}"#,
            ),
            (
                "data/pick/horizon/block/pillar.json",
                r#"{"template": "minecraft:stone", "item": false, "block_item": "pick:rod"}"#,
            ),
            (
                "data/pick/horizon/block/gem_ore.json",
                r#"{"template": "minecraft:stone", "item": false, "block_item": "minecraft:diamond"}"#,
            ),
            (
                "data/pick/horizon/block/ghost.json",
                r#"{"template": "minecraft:stone", "item": false}"#,
            ),
            (
                "data/pick/horizon/block/crop.json",
                r#"{"template": "minecraft:stone", "item": false, "pick_item": "minecraft:wheat_seeds"}"#,
            ),
            (
                "data/pick/horizon/block/gilded.json",
                r#"{"template": "minecraft:stone", "pick_item": "pick:rod"}"#,
            ),
            (
                "data/pick/horizon/item/rod.json",
                r#"{"template": "minecraft:stick"}"#,
            ),
        ],
    );
    let root = resource_root_fixture(
        &base,
        Path::new(""),
        std::slice::from_ref(&m),
        &scratch.join("cache"),
    )
    .unwrap();
    let registry = Assets::load(&root).unwrap().registry;
    let facts = BlockFacts::load(&root, registry).unwrap();
    let behaviours = Behaviours::load(registry, &root).unwrap();
    let item = |name: &str| {
        let state = registry.default_state(name).unwrap();
        facts.get(state).item.to_string()
    };
    assert_eq!(item("pick:plain"), "pick:plain");
    assert_eq!(item("pick:pillar"), "pick:rod");
    assert_eq!(item("pick:gem_ore"), "minecraft:diamond");
    assert_eq!(item("pick:ghost"), "minecraft:air");
    let pick = |name: &str| {
        let state = registry.default_state(name).unwrap();
        facts.get(state).pick_item.to_string()
    };
    // Without a `pick_item`, pick-block takes the block's item.
    assert_eq!(pick("pick:plain"), "pick:plain");
    assert_eq!(pick("pick:pillar"), "pick:rod");
    assert_eq!(pick("pick:ghost"), "minecraft:air");
    // With one, it takes that, and `Block.asItem` is unchanged.
    assert_eq!(pick("pick:crop"), "minecraft:wheat_seeds");
    assert_eq!(item("pick:crop"), "minecraft:air");
    assert_eq!(pick("pick:gilded"), "pick:rod");
    assert_eq!(item("pick:gilded"), "pick:gilded");
    let id = |name: &str| behaviours.table.block_by_name(name).unwrap();
    assert_eq!(&*behaviours.block_data(id("pick:pillar")).item, "pick:rod");
    assert_eq!(
        &*behaviours.block_data(id("pick:ghost")).item,
        "minecraft:air"
    );

    let missing = folder_mod(
        &scratch.join("lost"),
        &[
            ("horizon-mod.toml", &manifest("lost", "")),
            (
                "data/lost/horizon/block/pillar.json",
                r#"{"template": "minecraft:stone", "item": false, "block_item": "lost:rod"}"#,
            ),
        ],
    );
    let e = resource_root_fixture(
        &base,
        Path::new(""),
        std::slice::from_ref(&missing),
        &scratch.join("cache2"),
    )
    .err()
    .unwrap()
    .to_string();
    assert!(e.contains("lost:pillar") && e.contains("lost:rod"), "{e}");
    let missing = folder_mod(
        &scratch.join("gone"),
        &[
            ("horizon-mod.toml", &manifest("gone", "")),
            (
                "data/gone/horizon/block/crop.json",
                r#"{"template": "minecraft:stone", "item": false, "pick_item": "gone:seed"}"#,
            ),
        ],
    );
    let e = resource_root_fixture(
        &base,
        Path::new(""),
        std::slice::from_ref(&missing),
        &scratch.join("cache3"),
    )
    .err()
    .unwrap()
    .to_string();
    assert!(e.contains("gone:crop") && e.contains("gone:seed"), "{e}");
    let _ = fs::remove_dir_all(&scratch);
}

/// `WorldGenRegion.setBlock`: a feature that places a mod's sign leaves it its
/// block entity, which goes by the template's class and not the block's name.
#[test]
#[ignore = "daily"]
fn a_feature_gives_a_mods_sign_its_block_entity() {
    use crate::containers::BlockEntity;
    use crate::streaming::{ChunkGenerator, Proto};

    let Some(base) = prepared() else {
        return;
    };
    let scratch =
        std::env::temp_dir().join(format!("horizon-mod-sign-feature-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let m = folder_mod(
        &scratch.join("post"),
        &[
            ("horizon-mod.toml", &manifest("post", "")),
            (
                "data/post/horizon/block/notice.json",
                r#"{"template": "minecraft:oak_sign"}"#,
            ),
            (
                "data/post/worldgen/feature/notice.json",
                r#"{"type": "minecraft:ore", "size": 9, "discard_chance_on_air_exposure": 0.0,
                    "targets": [{"target": {"predicate_type": "minecraft:tag_match",
                        "tag": "minecraft:stone_ore_replaceables"},
                        "state": "post:notice"}]}"#,
            ),
            (
                "data/post/worldgen/placed_feature/notice.json",
                r#"{"feature": "post:notice", "placement": [
                    {"type": "minecraft:count", "count": 40}, {"type": "minecraft:in_square"},
                    {"type": "minecraft:height_range", "height": {"type": "minecraft:uniform",
                        "min_inclusive": {"absolute": -32}, "max_inclusive": {"absolute": 48}}},
                    {"type": "minecraft:biome"}]}"#,
            ),
            (
                "data/post/neoforge/biome_modifier/add_notice.json",
                r##"{"type": "neoforge:add_features", "biomes": "#minecraft:is_overworld",
                    "features": "post:notice", "step": "underground_ores"}"##,
            ),
        ],
    );
    let root = resource_root_fixture(&base, Path::new(""), &[m], &scratch.join("cache")).unwrap();
    let assets = Assets::load(&root).unwrap();
    let registry = assets.registry;
    let facts = BlockFacts::load(&root, registry).unwrap();
    let biomes = crate::biome::Biomes::load(
        &horizon_assets::resource_source::ResourceRoot::directory(root.to_path_buf()),
    )
    .unwrap();
    let item_data =
        crate::test_resources::item_data(&root, DataLoadOrigin::ServerDataLoad).unwrap();
    let terrain_resources = crate::vanillagen::TerrainResources {
        data: None,
        dir: &root.join("worldgen"),
        registry,
        biomes: &biomes,
        item_data: &item_data,
    };
    let notice = registry.default_state("post:notice").unwrap().0;
    let generate = |signs: bool| -> (usize, usize) {
        let mut terrain = crate::vanillagen::VanillaTerrain::new(terrain_resources, 1234).unwrap();
        if signs {
            terrain.add_mod_signs(&facts);
        }
        let mut worker = terrain.worker();
        let mut region: [Proto; 9] =
            std::array::from_fn(|i| terrain.proto(&mut worker, i as i32 % 3 - 1, i as i32 / 3 - 1));
        terrain.decorate(&mut worker, &mut region);
        let [_, _, _, _, center, ..] = region;
        let chunk = terrain.finish(center);
        let mut placed = 0;
        for y in chunk.min_y()..chunk.max_y() {
            for z in 0..16 {
                for x in 0..16 {
                    placed += usize::from(chunk.get(x, y, z) == notice);
                }
            }
        }
        let entities = chunk
            .block_entities
            .iter()
            .filter(|(_, e)| matches!(e, BlockEntity::Sign(s) if !s.hanging))
            .count();
        (placed, entities)
    };
    let (placed, entities) = generate(false);
    assert!(placed > 0, "the feature places the sign");
    assert_eq!(entities, 0, "by its name alone it has none");
    assert_eq!(generate(true), (placed, placed));
    let _ = fs::remove_dir_all(&scratch);
}

/// A small stand-in for the base root: nested folders, an empty file, an empty folder.
fn small_base(base: &Path) {
    for (name, bytes) in [
        ("items.json", &b"{}"[..]),
        ("empty.log", &b""[..]),
        ("assets/minecraft/lang/en_us.json", &b"{\"a\": 1}"[..]),
        (
            "data/minecraft/tags/block/logs.json",
            &b"{\"values\": []}"[..],
        ),
    ] {
        let path = base.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }
    fs::create_dir_all(base.join("worldgen/empty-folder")).unwrap();
}

/// Every folder (`None`) and file (`Some(bytes)`) under `root`, by relative path.
fn tree(root: &Path) -> BTreeMap<String, Option<Vec<u8>>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, Option<Vec<u8>>>) {
        for entry in fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if path.is_dir() {
                out.insert(rel, None);
                walk(root, &path, out);
            } else {
                out.insert(rel, Some(fs::read(&path).unwrap()));
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

/// Writes into a file where it is (same data), not by replacing it: what a link shares and a copy does not.
fn edit_in_place(path: &Path, bytes: &[u8]) {
    use std::io::Write;
    let mut file = fs::File::options()
        .write(true)
        .truncate(true)
        .open(path)
        .unwrap();
    file.write_all(bytes).unwrap();
}

/// The overlay looks like the base, and shares its files (a hard link on Windows, a symlink elsewhere) rather than
/// holding copies of them.
#[test]
fn the_overlay_shares_the_base_files() {
    let scratch = std::env::temp_dir().join(format!("horizon-mod-share-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let (base, out) = (scratch.join("base"), scratch.join("root"));
    small_base(&base);
    Overlay::new(&base, &out).unwrap();
    assert_eq!(tree(&out), tree(&base));
    edit_in_place(
        &base.join("assets/minecraft/lang/en_us.json"),
        b"{\"a\": 2}",
    );
    assert_eq!(
        fs::read(out.join("assets/minecraft/lang/en_us.json")).unwrap(),
        b"{\"a\": 2}",
        "the overlay copied the base file instead of linking it"
    );
    let _ = fs::remove_dir_all(&scratch);
}

/// Writing a file of the overlay replaces the name, so the base file it was linked to keeps its bytes and the other
/// files of the overlay still follow the base.
#[test]
fn writing_into_the_overlay_leaves_the_base_alone() {
    let scratch = std::env::temp_dir().join(format!("horizon-mod-write-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let (base, out) = (scratch.join("base"), scratch.join("root"));
    small_base(&base);
    let overlay = Overlay::new(&base, &out).unwrap();
    let before = tree(&base);
    overlay
        .write(
            "data/minecraft/tags/block/logs.json",
            b"{\"values\": [\"mod:log\"]}",
        )
        .unwrap();
    overlay.write("items.json", b"{\"mod\": 1}").unwrap();
    overlay.write("assets/mod/new.json", b"{}").unwrap();
    assert_eq!(tree(&base), before, "a write reached the base");
    assert_eq!(
        fs::read(out.join("data/minecraft/tags/block/logs.json")).unwrap(),
        b"{\"values\": [\"mod:log\"]}"
    );
    assert_eq!(fs::read(out.join("items.json")).unwrap(), b"{\"mod\": 1}");
    edit_in_place(
        &base.join("assets/minecraft/lang/en_us.json"),
        b"{\"a\": 3}",
    );
    assert_eq!(
        fs::read(out.join("assets/minecraft/lang/en_us.json")).unwrap(),
        b"{\"a\": 3}"
    );
    // A rebuild starts by removing the overlay: that removes names, not the base's data.
    fs::remove_dir_all(&out).unwrap();
    assert_eq!(fs::read(base.join("items.json")).unwrap(), b"{}");
    let _ = fs::remove_dir_all(&scratch);
}

/// Where a hard link cannot be made (another volume, a filesystem without links) a file is copied, as it always was
/// on Windows; where it can, the file is linked.
#[test]
fn a_file_is_linked_or_else_copied() {
    let scratch = std::env::temp_dir().join(format!("horizon-mod-linkfile-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    fs::create_dir_all(&scratch).unwrap();
    let target = scratch.join("base.json");
    fs::write(&target, b"original").unwrap();

    let linked = scratch.join("linked.json");
    link_file(&target, &linked, |from, to| fs::hard_link(from, to)).unwrap();
    edit_in_place(&target, b"edited");
    assert_eq!(fs::read(&linked).unwrap(), b"edited", "not a link");

    let copied = scratch.join("copied.json");
    fs::write(&target, b"original").unwrap();
    link_file(&target, &copied, |_, _| {
        Err(std::io::Error::other("no links here"))
    })
    .unwrap();
    edit_in_place(&target, b"edited");
    assert_eq!(fs::read(&copied).unwrap(), b"original", "not a copy");

    // A target that is not there is an error whichever way the file is made.
    let missing = link_file(
        &scratch.join("missing.json"),
        &scratch.join("x.json"),
        |from, to| fs::hard_link(from, to),
    );
    assert!(missing.is_err());
    let _ = fs::remove_dir_all(&scratch);
}

/// A symlink in the base is copied through, as before; linked, the overlay's file would be a symlink too.
#[cfg(unix)]
#[test]
fn a_symlink_file_is_copied_through() {
    let scratch = std::env::temp_dir().join(format!("horizon-mod-symfile-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    fs::create_dir_all(&scratch).unwrap();
    fs::write(scratch.join("real.json"), b"real").unwrap();
    std::os::unix::fs::symlink("real.json", scratch.join("alias.json")).unwrap();
    let at = scratch.join("copy.json");
    link_file(&scratch.join("alias.json"), &at, |from, to| {
        fs::hard_link(from, to)
    })
    .unwrap();
    assert!(!fs::symlink_metadata(&at).unwrap().file_type().is_symlink());
    assert_eq!(fs::read(&at).unwrap(), b"real");
    let _ = fs::remove_dir_all(&scratch);
}

/// A declared sign text colour is the index into the dye table the signs draw with.
#[test]
fn sign_text_colors_are_the_signs_dye_ids() {
    let names: Vec<_> = crate::signs::DYE_TEXT
        .iter()
        .map(|(name, _)| *name)
        .collect();
    assert_eq!(names, horizon_mod_package::content::DYE_COLORS);
}

/// A content mod of recipes, a tag and a name, for vanilla's items: what
/// switches live (docs/design/mod-hotload.md).
fn glint_mod(dir: &Path) -> ContentMod {
    folder_mod(
        dir,
        &[
            ("horizon-mod.toml", &manifest("glint", "")),
            (
                "data/glint/recipe/diamond_from_dirt.json",
                r#"{"type": "minecraft:crafting_shapeless", "category": "misc",
                    "ingredients": ["minecraft:dirt", "minecraft:dirt"],
                    "result": {"id": "minecraft:diamond"}}"#,
            ),
            (
                "data/c/tags/item/gems/glint.json",
                r#"{"values": ["minecraft:diamond"]}"#,
            ),
            (
                "assets/glint/lang/en_us.json",
                r#"{"glint.hello": "Hello"}"#,
            ),
        ],
    )
}

#[test]
fn only_assets_and_data_switch_live() {
    let scratch = std::env::temp_dir().join(format!("horizon-mod-live-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    assert_eq!(
        live_switch(&glint_mod(&scratch.join("glint")).content),
        Ok(())
    );
    let ruby = ruby_mod(&scratch.join("ruby"));
    assert_eq!(
        live_switch(&ruby.content),
        Err("horizon.mods.switch.registries")
    );
    let effect = folder_mod(
        &scratch.join("fx"),
        &[
            ("horizon-mod.toml", &manifest("fx", "")),
            (
                "data/fx/horizon/mob_effect/glow.json",
                r#"{"category": "harmful", "color": 1}"#,
            ),
        ],
    );
    assert_eq!(
        live_switch(&effect.content),
        Err("horizon.mods.switch.registries")
    );
    for (id, file, text) in [
        (
            "ores",
            "data/ores/neoforge/biome_modifier/add.json",
            r#"{"type": "neoforge:none"}"#,
        ),
        (
            "village",
            "data/minecraft/tags/worldgen/biome/has_structure/village_plains.json",
            r#"{"values": []}"#,
        ),
        (
            "feat",
            "data/feat/worldgen/placed_feature/rock.json",
            r#"{"feature": "minecraft:ore_iron", "placement": []}"#,
        ),
    ] {
        let m = folder_mod(
            &scratch.join(id),
            &[("horizon-mod.toml", &manifest(id, "")), (file, text)],
        );
        assert_eq!(
            live_switch(&m.content),
            Err("horizon.mods.switch.worldgen"),
            "{file}"
        );
    }
    fs::remove_dir_all(&scratch).unwrap();
}

/// On, off and on again: each root has what its set of mods has, a switch
/// never changes a root a game was given, and a set seen before reuses its
/// root without building it again.
#[test]
#[ignore = "daily"]
fn a_data_mod_loads_unloads_and_loads_again() {
    let Some(base) = prepared() else {
        return;
    };
    let scratch = std::env::temp_dir().join(format!("horizon-mod-reload-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let cache = scratch.join("cache");
    let glint = glint_mod(&scratch.join("glint"));
    let dirt = || Some(crate::items::ItemStack::new("minecraft:dirt", 1));
    let grid = [dirt(), dirt(), None, None];
    // What a world reads from a root: the item tags and the recipes.
    let tables = |root: &Path, mods: &[ContentMod]| {
        let resources = resolved_resources(
            &base,
            root.to_path_buf(),
            Path::new(""),
            mods,
            cache.clone(),
        )
        .unwrap();
        let files = content_files(&resources);
        let overlay = PackOverlay::new(&files);
        let assets = Assets::load(root).unwrap();
        let facts = BlockFacts::load(root, assets.registry).unwrap();
        let defaults = horizon_world::item_defaults::read(root).unwrap();
        let items = Items::load(root, assets.registry, &facts, &overlay, &defaults).unwrap();
        let recipes = crate::crafting::Recipes::load(
            root,
            &items,
            &overlay,
            &crate::test_resources::server_load(),
        )
        .unwrap();
        let crafted = recipes
            .find(crate::test_resources::server_loop_context(), &grid, 2)
            .map(|(_, out)| out.item.to_string());
        let tagged = items.item_tag_order.get("c:gems/glint").cloned();
        (
            crafted,
            tagged,
            resources.source().contains("assets/glint/lang/en_us.json"),
        )
    };
    let loaded = (
        Some("minecraft:diamond".to_owned()),
        Some(vec!["minecraft:diamond".to_owned()]),
        true,
    );

    let started = std::time::Instant::now();
    let on =
        resource_root_fixture(&base, Path::new(""), std::slice::from_ref(&glint), &cache).unwrap();
    eprintln!("content root built in {} ms", started.elapsed().as_millis());
    assert_ne!(on, base);
    assert_eq!(tables(&on, std::slice::from_ref(&glint)), loaded);

    // Off: the base root, without any of it.
    let off = resource_root_fixture(&base, Path::new(""), &[], &cache).unwrap();
    assert_eq!(off, base);
    assert_eq!(tables(&off, &[]), (None, None, false));
    assert!(!on.join("assets/glint/lang/en_us.json").exists());
    assert!(
        resolved_resources(
            &base,
            on.clone(),
            Path::new(""),
            std::slice::from_ref(&glint),
            cache.clone()
        )
        .unwrap()
        .source()
        .contains("assets/glint/lang/en_us.json")
    );

    // On again: the same root, not built again.
    let sentinel = on.join("sentinel");
    fs::write(&sentinel, b"").unwrap();
    let started = std::time::Instant::now();
    let again =
        resource_root_fixture(&base, Path::new(""), std::slice::from_ref(&glint), &cache).unwrap();
    eprintln!(
        "content root reused in {} ms",
        started.elapsed().as_millis()
    );
    assert_eq!(again, on);
    assert!(sentinel.exists());
    assert_eq!(tables(&again, std::slice::from_ref(&glint)), loaded);

    // Another set of mods builds its own root beside it and leaves this one as it was.
    let ruby = ruby_mod(&scratch.join("ruby"));
    let package = horizon_mod_package::Package::read(&scratch.join("glint")).unwrap();
    let glint_again = ContentMod {
        id: "glint".into(),
        jars: Vec::new(),
        content: Content::read(&scratch.join("glint"), &package.manifest).unwrap(),
    };
    let both_mods = [glint_again, ruby];
    let both = resource_root_fixture(&base, Path::new(""), &both_mods, &cache).unwrap();
    assert_ne!(both, on);
    assert!(sentinel.exists());
    assert_eq!(tables(&both, &both_mods).0, loaded.0);
    fs::remove_dir_all(&scratch).unwrap();
}

#[test]
fn roots_unused_for_a_day_are_removed() {
    let scratch = std::env::temp_dir().join(format!("horizon-mod-prune-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let roots = scratch.join("mod-content");
    let root = |name: &str, age_s: u64| {
        let dir = roots.join(name);
        fs::create_dir_all(&dir).unwrap();
        let stamp = dir.join(STAMP);
        fs::write(&stamp, b"x").unwrap();
        let file = fs::File::options().write(true).open(&stamp).unwrap();
        file.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(age_s))
            .unwrap();
        dir
    };
    let current = root("00000000000000aa", 3 * 86400);
    let recent = root("00000000000000bb", 60);
    let stale = root("00000000000000cc", 2 * 86400);
    // Built two days ago but reused a minute ago: its `.used` beside it keeps it (#12403).
    let reused = root("00000000000000dd", 2 * 86400);
    fs::write(used_path(&reused), b"").unwrap();
    fs::write(used_path(&stale), b"").unwrap();
    let file = fs::File::options()
        .write(true)
        .open(used_path(&stale))
        .unwrap();
    file.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(2 * 86400))
        .unwrap();
    drop(file);
    // Not a root's name: never touched while the folder has no root of the old layout.
    let other = roots.join("notes");
    fs::create_dir_all(&other).unwrap();
    prune_roots(&roots, &current, KEEP_ROOTS);
    assert!(current.exists() && recent.exists() && other.exists() && reused.exists());
    assert!(!stale.exists() && !used_path(&stale).exists());
    // An older game's single root, built straight in the folder, goes.
    fs::write(roots.join(STAMP), b"old").unwrap();
    fs::create_dir_all(roots.join("assets/minecraft")).unwrap();
    prune_roots(&roots, &current, KEEP_ROOTS);
    assert!(!roots.join(STAMP).exists() && !roots.join("assets").exists());
    assert!(current.exists() && recent.exists());
    fs::remove_dir_all(&scratch).unwrap();
}

// These fixtures accept every listed content package on both sides. Production
// captures the inventory after admission, including active code-only packages.
fn fixture_inventory(mods: &[ContentMod]) -> horizon_mod_package::enabled::EnabledMods {
    horizon_mod_package::enabled::EnabledMods::new(
        mods.iter()
            .map(|package| (package.id.clone(), mod_api::Side::Both)),
    )
}
fn build_fixture(base: &Path, jars: &Path, mods: &[ContentMod], out: &Path) -> Result<Vec<String>> {
    super::build(base, jars, mods, out, &fixture_inventory(mods))
}
fn resource_root_fixture(
    base: &Path,
    jars: &Path,
    mods: &[ContentMod],
    cache: &Path,
) -> Result<PathBuf> {
    super::resource_root(base, jars, mods, cache, &fixture_inventory(mods))
}

#[path = "mod_content_load_conditions_tests.rs"]
mod registered_conditions_tests;
