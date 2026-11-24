use crate::mobs::spawner::SpawnDataExt;

/// A bare file name lives in the working directory: syncing its folder syncs `.`, not the
/// empty path, so a durable write there that renamed its file into place also succeeds.
#[test]
fn a_bare_file_name_syncs_the_working_directory() {
    super::sync_folder(std::path::Path::new("level-not-written.json")).unwrap();
}

#[test]
fn transient_mod_tickets_hold_chunks_without_reopening_them_on_restart() {
    use super::*;
    let root = tempfile::tempdir().unwrap();
    let save = SaveDir::new(root.path().to_path_buf(), LevelShape::OVERWORLD);
    let mut tickets = ModTickets::default();
    let block = ModTicket {
        controller: 0,
        owner: TicketOwner::Block([1, -2, 3]),
        chunk: (4, 5),
        natural_spawning: true,
    };
    let entity = ModTicket {
        owner: TicketOwner::Entity(0x1234_5678_9abc_def0_1122_3344_5566_7788),
        chunk: (6, 7),
        natural_spawning: false,
        ..block.clone()
    };
    tickets.insert_transient(crate::mod_chunk_tickets::TransientTicket {
        controller: 0,
        operation: 1,
        chunk: (8, 9),
        natural_spawning: false,
    });
    tickets.insert(block.clone());
    tickets.insert(entity.clone());
    assert!(tickets.holds((8, 9)));
    save.save_mod_tickets(&tickets, |_| Some("test:teleport"))
        .unwrap();
    let mut loaded = save.load_mod_tickets(|_| Some(0));
    loaded.sort();
    assert_eq!(loaded, [block, entity]);
}

#[test]
fn saved_preset_editor_stems_supply_shapes_and_facts_below_pack_stems() {
    use super::*;
    use horizon_world::worldgen::overrides::{DIMENSIONS, WorldgenOverrides};
    let dir = std::env::temp_dir().join(format!(
        "horizon-preset-shapes-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let dimensions = json_tag(&serde_json::json!({
        "minecraft:overworld": {"type":"minecraft:the_nether","generator":{"type":"minecraft:debug"}},
        "example:pocket": {
            "type":{"min_y":-128,"height":448,"logical_height":320,"has_skylight":false,"has_ceiling":true},
            "generator":{"type":"minecraft:debug"}
        }
    }));
    write_world_gen_settings_with(&dir, 1, WorldGen::default(), None, None, Some(dimensions))
        .unwrap();
    let mut overrides = WorldgenOverrides::default();
    merge_saved_world_gen_dimensions(&dir, &mut overrides);
    let overworld = overrides.level_height("minecraft:overworld").unwrap();
    assert_eq!((overworld.min_y, overworld.height), (0, 256));
    assert!(overworld.has_ceiling && !overworld.has_skylight);
    let pocket = overrides.level_height("example:pocket").unwrap();
    assert_eq!(
        (pocket.min_y, pocket.height, pocket.logical_height),
        (-128, 448, 320)
    );
    assert!(pocket.has_ceiling && !pocket.has_skylight);
    assert!(
        overrides
            .level_facts("minecraft:overworld")
            .unwrap()
            .unwrap()
            .fast_lava
    );
    let pack_stem = r#"{"type":"minecraft:overworld","generator":{"type":"minecraft:debug"}}"#;
    overrides.set(DIMENSIONS, "minecraft:overworld", pack_stem.into());
    merge_saved_world_gen_dimensions(&dir, &mut overrides);
    assert_eq!(
        overrides
            .entries(DIMENSIONS)
            .find(|(id, _)| *id == "minecraft:overworld")
            .unwrap()
            .1,
        pack_stem
    );
    assert_eq!(
        overrides.level_height("minecraft:overworld").unwrap().min_y,
        -64
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn an_ender_chest_saves_none_of_the_players_items() {
    use crate::containers::BlockEntity;
    let mut slots = vec![None; 27];
    slots[0] = Some(crate::items::ItemStack::new("minecraft:diamond", 3));
    let chest = super::block_entity_encode::encode(
        (1, 2, 3),
        &BlockEntity::Chest(slots.clone()),
        "minecraft:chest",
    );
    assert!(
        chest.get("Items").is_some(),
        "control: a chest saves its items"
    );
    let ender = super::block_entity_encode::encode(
        (1, 2, 3),
        &BlockEntity::Chest(slots),
        "minecraft:ender_chest",
    );
    assert_eq!(
        ender.get("id"),
        Some(&super::string("minecraft:ender_chest"))
    );
    assert!(ender.get("Items").is_none(), "{ender:?}");
}
#[test]
fn registered_rule_survives_level_save_and_absence() {
    use crate::gamerules::{Rule, RuleRegistry, Value};
    let dir = std::env::temp_dir().join(format!("horizon-mod-rule-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let rule = Rule {
        name: "example:enabled".into(),
        category: "example:machines".into(),
        default: Value::Bool(true),
        min: 0,
        max: 0,
    };
    let registry = std::sync::Arc::new(
        RuleRegistry::with_mod_rules(&["example:machines".into()], &[rule]).unwrap(),
    );
    let mut level = super::Level {
        rule_registry: registry.clone(),
        ..Default::default()
    };
    level.game_rules.insert("example:enabled".into(), 0);
    level.write(&dir).unwrap();
    assert_eq!(
        super::read_saved_data(&dir, "game_rules")
            .unwrap()
            .get("example:enabled"),
        Some(&super::Tag::Byte(0))
    );
    let mut absent = super::Level::read(&dir).unwrap();
    absent.write(&dir).unwrap();
    assert_eq!(
        super::read_saved_data(&dir, "game_rules")
            .unwrap()
            .get("example:enabled"),
        Some(&super::Tag::Byte(0))
    );
    let mut returned = super::Level::read_with_registry(&dir, registry).unwrap();
    assert_eq!(returned.game_rules.get("example:enabled"), Some(&0));
    returned.write(&dir).unwrap();
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_server_falls_back_to_level_dat_old_and_restores_it() {
    let world = std::env::temp_dir().join(format!("horizon-dat-old-{}", std::process::id()));
    let _ = fs::remove_dir_all(&world);
    fs::create_dir_all(&world).unwrap();
    let level = |name: &str| {
        Tag::compound([(
            "Data",
            Tag::compound([("LevelName", Tag::String(name.into()))]),
        )])
    };
    // Both files broken, or only level.dat_old there: nothing to fall back to.
    fs::write(world.join("level.dat"), b"cut short").unwrap();
    assert!(!fall_back_to_level_dat_old(&world));
    fs::write(world.join("level.dat_old"), b"cut short").unwrap();
    assert!(!fall_back_to_level_dat_old(&world));
    // A good level.dat_old: it takes level.dat's place and the broken file is kept.
    write_dat(&world.join("level.dat_old"), level("old")).unwrap();
    assert!(fall_back_to_level_dat_old(&world));
    assert!(level_dat_issue(&world, false).is_none());
    let kept = fs::read_dir(&world)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("level.dat_corrupted_")
        })
        .count();
    assert_eq!(kept, 1);
    // A readable level.dat is left alone.
    assert!(!fall_back_to_level_dat_old(&world));
    let _ = fs::remove_dir_all(&world);
}

#[test]
fn points_of_interest_save_as_poi_sections() {
    let Some(root) = std::env::var_os("HORIZON_TEST_RESOURCES") else {
        return;
    };
    let assets = crate::assets::Assets::load(&PathBuf::from(root)).expect("assets");
    let tables = SaveTables::new(&assets);
    let state = |name: &str, props: &[(&str, &str)]| {
        let mut s = assets
            .registry
            .state(assets.registry.default_state(name).unwrap())
            .unwrap()
            .clone();
        for (k, v) in props {
            s.properties.insert((*k).into(), (*v).into());
        }
        assets.registry.id(&s).unwrap().0
    };
    let mut chunk = Chunk::empty(2, -1);
    chunk.set(1, 70, 1, state("minecraft:red_bed", &[("part", "head")]));
    chunk.set(1, 70, 2, state("minecraft:red_bed", &[("part", "foot")]));
    chunk.set(5, 71, 5, state("minecraft:bell", &[]));
    chunk.set(6, 71, 5, state("minecraft:stone", &[]));
    chunk.set(9, 70, 9, state("minecraft:straw_bed", &[("part", "head")]));
    assert!(encode_poi(&Chunk::empty(0, 0), &tables, None).is_none());
    let saved = encode_poi(&chunk, &tables, None).expect("records");
    let Some(Tag::Compound(sections)) = saved.tag.get("Sections") else {
        panic!("no sections");
    };
    assert_eq!(sections.len(), 1);
    let section = &sections[&"4".into()];
    assert_eq!(section.get("Valid"), Some(&Tag::Byte(1)));
    let Some(Tag::List(records)) = section.get("Records") else {
        panic!("no records");
    };
    let records: Vec<_> = records
        .iter()
        .map(|r| {
            (
                text(r.get("type")).unwrap(),
                r.get("pos").cloned(),
                number(r.get("free_tickets")),
            )
        })
        .collect();
    assert_eq!(
        records,
        [
            (
                "minecraft:home".into(),
                Some(Tag::IntArray(vec![33, 70, -15])),
                Some(1)
            ),
            (
                "minecraft:meeting".into(),
                Some(Tag::IntArray(vec![37, 71, -11])),
                Some(32)
            ),
        ]
    );
}

/// A block a mod added to `minecraft:home` (`extend_poi_types`) saves as a
/// home record, as a bed head does.
#[test]
fn states_a_mod_added_to_a_poi_type_save_as_its_records() {
    let Some(root) = std::env::var_os("HORIZON_TEST_RESOURCES") else {
        return;
    };
    let assets = crate::assets::Assets::load(&PathBuf::from(root)).expect("assets");
    let hay_states: Vec<_> = (0..assets.registry.len())
        .filter_map(|i| assets.registry.state(BlockStateId(i as u32)))
        .filter(|state| state.name == "minecraft:hay_block")
        .map(|state| horizon_world::poi::PoiExtension {
            state: state.clone(),
            poi: "minecraft:home".into(),
        })
        .collect();
    assert_eq!(hay_states.len(), 3);
    // The tables are built once per registry, so they keep the addition.
    let tables = horizon_world::poi::with_extensions(hay_states, |refusals| {
        assert_eq!(refusals, Vec::<String>::new());
        SaveTables::new(&assets)
    });
    let hay = assets
        .registry
        .default_state("minecraft:hay_block")
        .unwrap();
    assert_eq!(tables.poi[hay.0 as usize], Some(("minecraft:home", 1)));
    let stone = assets.registry.default_state("minecraft:stone").unwrap();
    assert_eq!(tables.poi[stone.0 as usize], None);
    let mut chunk = Chunk::empty(0, 0);
    chunk.set(3, 70, 3, hay.0);
    chunk.set(4, 70, 3, stone.0);
    let saved = encode_poi(&chunk, &tables, None).expect("records");
    let Some(Tag::Compound(sections)) = saved.tag.get("Sections") else {
        panic!("no sections");
    };
    let Some(Tag::List(records)) = sections[&"4".into()].get("Records") else {
        panic!("no records");
    };
    let records: Vec<_> = records
        .iter()
        .map(|r| (text(r.get("type")).unwrap(), r.get("pos").cloned()))
        .collect();
    assert_eq!(
        records,
        [("minecraft:home".into(), Some(Tag::IntArray(vec![3, 70, 3])))]
    );
}

#[test]
fn a_chunk_that_loses_its_last_poi_saves_empty_sections() {
    let Some(root) = std::env::var_os("HORIZON_TEST_RESOURCES") else {
        return;
    };
    let root = PathBuf::from(root);
    let assets = crate::assets::Assets::load(&root).expect("assets");
    let biomes = Arc::new(
        Biomes::load(&horizon_assets::resource_source::ResourceRoot::directory(
            root.to_path_buf(),
        ))
        .expect("biomes"),
    );
    let bell = assets.registry.default_state("minecraft:bell").unwrap().0;
    let dir = std::env::temp_dir().join(format!("horizon-poi-save-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let save = SaveDir::new(dir.clone(), LevelShape::OVERWORLD);
    let _ = save.tables.set(Arc::new(SaveTables::new(&assets)));
    let mut chunk = Chunk::empty(0, 0);
    chunk.set(5, 71, 5, bell);
    let sections = |save: &SaveDir| match save.read_record("poi", 0, 0)?.tag.get("Sections") {
        Some(Tag::Compound(s)) => Some(s.len()),
        _ => None,
    };
    save.save_later(&[Arc::new(chunk.clone())], assets.registry, &biomes)
        .unwrap();
    save.flush().unwrap();
    assert_eq!(sections(&save), Some(1));
    chunk.set(5, 71, 5, 0);
    save.save_later(&[Arc::new(chunk)], assets.registry, &biomes)
        .unwrap();
    save.flush().unwrap();
    assert_eq!(
        sections(&SaveDir::new(dir.clone(), LevelShape::OVERWORLD)),
        Some(0)
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn saved_poi_records_keep_their_claims() {
    let record = |pos: [i32; 3], kind: &str, tickets: Option<i32>| {
        let mut r = vec![("pos", Tag::IntArray(pos.to_vec())), ("type", string(kind))];
        if let Some(t) = tickets {
            r.push(("free_tickets", Tag::Int(t)));
        }
        Tag::compound(r)
    };
    let poi = |records: Vec<Tag>| {
        let mut sections = Compound::new();
        sections.insert(
            "4".into(),
            Tag::compound([("Valid", Tag::Byte(1)), ("Records", Tag::List(records))]),
        );
        poi_root(sections)
    };
    let old = poi(vec![
        record([1, 70, 1], "minecraft:home", None),
        record([5, 71, 5], "minecraft:meeting", Some(30)),
    ]);
    let mut saved = poi(vec![
        record([1, 70, 1], "minecraft:home", Some(1)),
        record([5, 71, 5], "minecraft:meeting", Some(32)),
        record([9, 71, 9], "minecraft:home", Some(1)),
    ]);
    keep_claims(&mut saved, &old);
    let expected = poi(vec![
        record([1, 70, 1], "minecraft:home", None),
        record([5, 71, 5], "minecraft:meeting", Some(30)),
        record([9, 71, 9], "minecraft:home", Some(1)),
    ]);
    assert_eq!(saved.tag, expected.tag);
}

#[test]
fn level_attachments_survive_save_and_load() {
    use horizon_mod_runtime::attachments::{
        AttachmentCodec, AttachmentError, AttachmentType, Attachments,
    };
    let dir =
        std::env::temp_dir().join(format!("horizon-level-attachments-{}", std::process::id()));
    let save = SaveDir::new(dir.clone(), LevelShape::OVERWORLD);
    let key = AttachmentType {
        namespace: "test",
        name: "counter",
        default: || 0,
        codec: Some(AttachmentCodec {
            encode: |n: &i32| Ok([("count".into(), Tag::Int(*n))].into()),
            decode: |c: &Compound| match c.get(&"count".into()) {
                Some(Tag::Int(n)) => Ok(*n),
                _ => Err(AttachmentError::Decode),
            },
        }),
        copy_on_death: false,
        sync: false,
    };
    let mut map = Attachments::default();
    key.set(&mut map, horizon_mod_runtime::attachments::Holder::Level, 7)
        .unwrap();
    save.save_attachments(&map).unwrap();
    assert_eq!(
        key.get(
            &mut save.load_attachments(),
            horizon_mod_runtime::attachments::Holder::Level
        ),
        Ok(7)
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[ignore = "daily"]
fn loaded_chunk_uses_saved_monument_from_its_referenced_start_chunk() {
    use horizon_core::{
        geometry::Direction,
        random::{LegacyRandom, WorldgenRandom},
    };
    use horizon_world::worldgen::structure::{
        legacy::{LegacyPiece, random_horizontal},
        monument::MonumentBuilding,
    };
    let root = PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").expect("resources"));
    let assets = Arc::new(crate::assets::Assets::load(&root).expect("assets"));
    let biomes = Arc::new(
        Biomes::load(&horizon_assets::resource_source::ResourceRoot::directory(
            root.clone(),
        ))
        .expect("biomes"),
    );
    let item_data = crate::test_resources::item_data(
        &root,
        horizon_world::native_load_context::DataLoadOrigin::ServerDataLoad,
    )
    .expect("item data");
    let inner = Arc::new(
        crate::vanillagen::VanillaTerrain::new(
            crate::vanillagen::TerrainResources {
                data: None,
                dir: &root.join("worldgen"),
                registry: assets.registry,
                biomes: &biomes,
                item_data: &item_data,
            },
            12345,
        )
        .unwrap(),
    );
    // A moved, rotated monument that generation cannot reproduce at the owning chunk.
    // Vanilla reseeds, draws the default even with a saved orientation, then rebuilds.
    let mut random = WorldgenRandom::new(LegacyRandom::new(0));
    random.set_large_feature_seed(12345, -2, 3);
    random_horizontal(&mut random);
    let top = MonumentBuilding::new(&mut random, 100, -300, Direction::East);
    let expected = Tag::compound([
        ("id", string("minecraft:monument")),
        ("ChunkX", Tag::Int(-2)),
        ("ChunkZ", Tag::Int(3)),
        ("references", Tag::Int(5)),
        (
            "Children",
            Tag::List(vec![Tag::Compound(top.save().unwrap())]),
        ),
    ]);
    let mut saved = encode(&Chunk::empty(-2, 3), assets.registry, &biomes);
    let Some(Tag::Compound(structures)) = saved.tag.get_mut("structures") else {
        panic!("structures")
    };
    structures.insert(
        "starts".into(),
        Tag::compound([("minecraft:monument", expected.clone())]),
    );
    let save = Arc::new(SaveDir::new(
        std::env::temp_dir().join(format!("horizon-saved-monument-{}", std::process::id())),
        LevelShape::OVERWORLD,
    ));
    save.pending
        .lock()
        .unwrap()
        .insert((Folder::Region, -2, 3), (0, Record::Tag(Arc::new(saved))));
    let generator = Persistent {
        inner: inner.clone(),
        save,
        registry: assets,
        biomes,
    };
    let mut worker = generator.worker();
    let mut referring = Chunk::empty(-1, 3);
    referring
        .structure_refs
        .push(("minecraft:monument".into(), (-2, 3)));
    generator.prepare_loaded(&mut worker, &referring);
    let starts = inner.starts_at(-2, 3);
    let restored = starts
        .iter()
        .find(|s| s.name == "minecraft:monument")
        .expect("referenced saved monument");
    assert_eq!(restored.save(), Some(expected));
}

#[test]
fn loaded_chunk_uses_saved_jigsaw_from_its_referenced_start_chunk() {
    let Some(root) = std::env::var_os("HORIZON_TEST_RESOURCES") else {
        return;
    };
    let root = PathBuf::from(root);
    let assets = Arc::new(crate::assets::Assets::load(&root).expect("assets"));
    let biomes = Arc::new(
        Biomes::load(&horizon_assets::resource_source::ResourceRoot::directory(
            root.clone(),
        ))
        .expect("biomes"),
    );
    let item_data = crate::test_resources::item_data(
        &root,
        horizon_world::native_load_context::DataLoadOrigin::ServerDataLoad,
    )
    .expect("item data");
    let inner = Arc::new(
        crate::vanillagen::VanillaTerrain::new(
            crate::vanillagen::TerrainResources {
                data: None,
                dir: &root.join("worldgen"),
                registry: assets.registry,
                biomes: &biomes,
                item_data: &item_data,
            },
            12345,
        )
        .unwrap(),
    );
    let mut original = inner
        .structure_start("minecraft:ancient_city", 2, -3)
        .unwrap();
    Arc::get_mut(&mut original).unwrap().references = 4;
    let starts = Tag::compound([
        ("minecraft:ancient_city", original.save().unwrap()),
        (
            "minecraft:mineshaft",
            Tag::compound([("id", string("INVALID"))]),
        ),
    ]);
    let mut saved = encode(&Chunk::empty(2, -3), assets.registry, &biomes);
    let Tag::Compound(root) = &mut saved.tag else {
        panic!("chunk root")
    };
    let Some(Tag::Compound(structures)) = root.get_mut(&"structures".into()) else {
        panic!("structures")
    };
    structures.insert("starts".into(), starts.clone());
    let loaded = decode(&saved, assets.registry, &biomes).expect("chunk");
    let Tag::Compound(expected) = &starts else {
        unreachable!()
    };
    assert_eq!(loaded.saved_structure_starts(), Some(expected));
    let again = encode(&loaded, assets.registry, &biomes);
    assert_eq!(
        again.tag.get("structures").and_then(|s| s.get("starts")),
        Some(&starts)
    );
    let save = Arc::new(SaveDir::new(
        std::env::temp_dir().join(format!("horizon-saved-starts-{}", std::process::id())),
        LevelShape::OVERWORLD,
    ));
    // A pending record is authoritative even before any region file exists.
    save.pending
        .lock()
        .unwrap()
        .insert((Folder::Region, 2, -3), (0, Record::Tag(Arc::new(saved))));
    let generator = Persistent {
        inner: inner.clone(),
        save,
        registry: assets,
        biomes,
    };
    let mut worker = generator.worker();
    let mut referring = Chunk::empty(3, -3);
    referring
        .structure_refs
        .push(("minecraft:ancient_city".into(), (2, -3)));
    generator.prepare_loaded(&mut worker, &referring);
    let starts = inner.starts_at(2, -3);
    let restored = starts
        .iter()
        .find(|s| s.name == original.name)
        .expect("referenced saved start");
    assert_eq!(restored.save(), original.save());
    assert_eq!(format!("{restored:?}"), format!("{original:?}"));
}

/// The three scattered-feature starts (desert pyramid, jungle temple,
/// buried treasure) are generated by the worldgen code, written through
/// the level's own chunk writer, read back with its reader, and the
/// generation that continues from them places the same chests, traps and
/// loot tables. A saved start that loses its state would regenerate the
/// loot or the buried-treasure search.
#[test]
fn saved_scattered_starts_reload_chests_traps_and_continue_placement() {
    use super::*;
    use crate::streaming::Proto;
    use crate::vanillagen::{TerrainResources, VanillaTerrain};
    use horizon_world::native_load_context::DataLoadOrigin;
    let Some(root) = std::env::var_os("HORIZON_TEST_RESOURCES") else {
        return;
    };
    let root = PathBuf::from(root);
    if !root.join("worldgen/world_presets.json").exists() {
        eprintln!("skipping: no worldgen bundle at {}", root.display());
        return;
    }
    let assets = Arc::new(crate::assets::Assets::load(&root).expect("assets"));
    let biomes = Arc::new(
        Biomes::load(&horizon_assets::resource_source::ResourceRoot::directory(
            root.clone(),
        ))
        .expect("biomes"),
    );
    let item_data =
        crate::test_resources::item_data(&root, DataLoadOrigin::ServerDataLoad).expect("item data");
    let worldgen = root.join("worldgen");
    let resources = TerrainResources {
        data: None,
        dir: &worldgen,
        registry: assets.registry,
        biomes: &biomes,
        item_data: &item_data,
    };
    // The names, their placed loot, and the chest/trap flags a placed
    // start carries.
    let specs: [(&str, &[(&str, usize)], &[&str]); 3] = [
        (
            "minecraft:desert_pyramid",
            &[("minecraft:chests/desert_pyramid", 4)],
            &[
                "hasPlacedChest0",
                "hasPlacedChest1",
                "hasPlacedChest2",
                "hasPlacedChest3",
            ],
        ),
        (
            "minecraft:jungle_pyramid",
            &[
                ("minecraft:chests/jungle_temple", 2),
                ("minecraft:chests/jungle_temple_dispenser", 2),
            ],
            &[
                "placedMainChest",
                "placedHiddenChest",
                "placedTrap1",
                "placedTrap2",
            ],
        ),
        (
            "minecraft:buried_treasure",
            &[("minecraft:chests/buried_treasure", 1)],
            &[],
        ),
    ];
    let inner = Arc::new(VanillaTerrain::new(resources, 1234).expect("terrain"));
    let start_chunk = |name: &str| -> ChunkPos {
        let names = [name.to_owned()];
        // A placement-set start, as `/locate structure` and an exploration
        // map find one: generated by the worldgen code, not written here.
        for radius in [64, 256, 1024] {
            if let Some((found, at)) = inner.locate(&names, [0, 64, 0], radius) {
                assert_eq!(found, name);
                return ChunkPos::new(at[0] >> 4, at[2] >> 4);
            }
        }
        panic!("no {name} start near the origin");
    };
    let loot_table = |tag: &Tag| -> Option<String> {
        let Tag::Compound(map) = tag else { return None };
        let Tag::String(table) = map.get(&"LootTable".into())? else {
            return None;
        };
        Some(table.to_string_lossy())
    };
    let entities = |list: &[Tag], table: &str| -> Vec<Tag> {
        list.iter()
            .filter(|tag| loot_table(tag).as_deref() == Some(table))
            .cloned()
            .collect()
    };
    let flag = |tag: &Compound, key: &str| -> bool {
        matches!(tag.get(&key.into()), Some(Tag::Byte(v)) if *v != 0)
    };
    // The dimensions, settled height and chest/trap flags are the first
    // piece's fields, not the start's.
    fn piece_tag(start: &Compound) -> &Compound {
        match start.get(&"Children".into()) {
            Some(Tag::List(children)) => match children.first() {
                Some(Tag::Compound(piece)) => piece,
                _ => panic!("start has a piece"),
            },
            _ => panic!("start has children"),
        }
    }
    let raw_start = |proto: &SavedProto, name: &str| -> Compound {
        match proto
            .column
            .saved_structure_starts()
            .and_then(|starts| starts.get(&name.into()).cloned())
        {
            Some(Tag::Compound(map)) => map,
            other => panic!("{name}: no saved start: {other:?}"),
        }
    };
    let typed_start = |proto: &SavedProto, name: &str| -> Compound {
        match proto
            .column
            .structure_starts
            .iter()
            .find(|start| start.name == name)
            .and_then(|start| start.save())
        {
            Some(Tag::Compound(map)) => map,
            other => panic!("{name}: no generated start: {other:?}"),
        }
    };
    let save_dir = |which: &str, index: usize| -> PathBuf {
        std::env::temp_dir().join(format!(
            "horizon-saved-scattered-{which}-{}-{index}",
            std::process::id()
        ))
    };
    let write_protos = |dir: &Path, protos: Vec<SavedProto>| {
        let save = SaveDir::new(dir.to_path_buf(), LevelShape::OVERWORLD);
        let _ = save.tables.set(Arc::new(SaveTables::new(&assets)));
        save.save_protos(protos, assets.registry, &biomes)
            .expect("save proto chunks");
        save.flush().expect("flush");
    };
    for (index, (name, loot, flags)) in specs.into_iter().enumerate() {
        let at = start_chunk(name);
        let (sx, sz) = (at.x, at.z);
        // The region at STATUS terrain: generation has made the start,
        // nothing has been placed yet.
        let mut worker = inner.worker();
        let terrain: [Proto; 9] = std::array::from_fn(|i| {
            inner.proto(&mut worker, sx + i as i32 % 3 - 1, sz + i as i32 / 3 - 1)
        });
        let before_region: Vec<SavedProto> = terrain
            .iter()
            .map(|proto| inner.saved_proto(proto).expect("proto"))
            .collect();
        let before = typed_start(&before_region[4], name);
        let start = before_region[4]
            .column
            .structure_starts
            .iter()
            .find(|start| start.name == name)
            .unwrap_or_else(|| panic!("{name} is the start of its chunk"));
        let bounds = start
            .pieces
            .iter()
            .map(|piece| piece.bounding_box())
            .reduce(|mut a, b| {
                a.encapsulate(&b);
                a
            })
            .expect("pieces");
        // Save the proto chunks and read the same record back. The
        // start is marked referenced as an exploration map that found it
        // would: a regenerated start has no references, a loaded one does.
        let mut before = before;
        before.insert("references".into(), Tag::Int(1));
        let first = save_dir("first", index);
        let save = SaveDir::new(first.clone(), LevelShape::OVERWORLD);
        let _ = save.tables.set(Arc::new(SaveTables::new(&assets)));
        let tables = save.tables.get().cloned().expect("save tables");
        let mut records = Vec::new();
        for (i, proto) in before_region.iter().enumerate() {
            let mut root = encode_proto(
                proto,
                assets.registry,
                &biomes,
                Some(tables.as_ref()),
                LevelShape::OVERWORLD,
            );
            if i == 4 {
                let Tag::Compound(root) = &mut root.tag else {
                    panic!("{name}: root")
                };
                let Tag::Compound(structures) =
                    root.get_mut(&"structures".into()).expect("structures")
                else {
                    panic!("{name}: structures")
                };
                let Tag::Compound(starts) = structures.get_mut(&"starts".into()).expect("starts")
                else {
                    panic!("{name}: starts")
                };
                let Tag::Compound(start) = starts.get_mut(&name.into()).expect("start") else {
                    panic!("{name}: start")
                };
                start.insert("references".into(), Tag::Int(1));
            }
            records.push(((proto.column.x, proto.column.z), Some(root)));
        }
        save.write_records("region", records, true)
            .expect("write saved proto chunks");
        drop(save);
        let reader = SaveDir::new(first.clone(), LevelShape::OVERWORLD);
        let _ = reader.tables.set(Arc::new(SaveTables::new(&assets)));
        let loaded = reader
            .load_proto(sx, sz, assets.registry, &biomes)
            .expect("saved proto chunk");
        assert!(!loaded.decorated, "{name}: saved before decoration");
        assert_eq!(
            raw_start(&loaded, name),
            before,
            "{name}: the saved start reads back before generation continues"
        );
        // A fresh generator over the save continues the statuses: the
        // saved start is loaded and its pieces place chests and traps.
        let reopened = Arc::new(VanillaTerrain::new(resources, 1234).expect("terrain"));
        let generator = Persistent {
            inner: reopened.clone(),
            save: Arc::new(reader),
            registry: assets.clone(),
            biomes: biomes.clone(),
        };
        let mut worker = generator.worker();
        let mut region: [Proto; 9] = std::array::from_fn(|i| {
            generator.proto(&mut worker, sx + i as i32 % 3 - 1, sz + i as i32 / 3 - 1)
        });
        generator.decorate(&mut worker, &mut region);
        let placed = reopened
            .saved_proto(&region[4])
            .unwrap_or_else(|| panic!("{name}: decorated proto chunk"));
        assert!(placed.decorated, "{name}: generation continued");
        let placed_start = typed_start(&placed, name);
        for &(table, count) in loot {
            let found = entities(&placed.block_entities, table);
            assert_eq!(
                found.len(),
                count,
                "{name}: {table} placed after loading the saved start"
            );
            for entity in &found {
                let Tag::Compound(map) = entity else {
                    unreachable!()
                };
                let pos = ["x", "y", "z"].map(|key| match map.get(&key.into()) {
                    Some(Tag::Int(v)) => *v,
                    other => panic!("{name}: {table} has {key}: {other:?}"),
                });
                // Chests and traps sit inside the piece's footprint;
                // some (the pyramid cellars) are below its saved box.
                assert!(
                    bounds.min[0] <= pos[0]
                        && pos[0] <= bounds.max[0]
                        && bounds.min[2] <= pos[2]
                        && pos[2] <= bounds.max[2],
                    "{name}: {table} at {pos:?} outside its pieces' footprint {bounds:?}"
                );
            }
        }
        assert_eq!(
            placed_start.get(&"references".into()),
            Some(&Tag::Int(1)),
            "{name}: placement uses the loaded start's references"
        );
        for &key in flags {
            assert!(
                flag(piece_tag(&placed_start), key),
                "{name}: {key} after placement"
            );
        }
        // The dimensions survive placement; the settled height is read
        // only after placement, and compared across the second save below.
        let before_piece = piece_tag(&before);
        let placed_piece = piece_tag(&placed_start);
        for key in ["Width", "Height", "Depth"] {
            assert_eq!(
                placed_piece.get(&key.into()),
                before_piece.get(&key.into()),
                "{name}: {key} survives save, load and placement"
            );
        }
        if name != "minecraft:buried_treasure" {
            assert!(
                matches!(placed_piece.get(&"HPos".into()), Some(Tag::Int(h)) if *h >= 0),
                "{name}: placement settled the height it saves"
            );
        }
        // Save the decorated proto chunks: block entities carry the loot
        // table and its seed, so a regenerated chest would differ.
        let decorated: Vec<SavedProto> = region
            .iter()
            .map(|proto| reopened.saved_proto(proto).expect("decorated proto"))
            .collect();
        let center_entities = decorated[4].block_entities.clone();
        let second = save_dir("second", index);
        write_protos(&second, decorated);
        let reader = SaveDir::new(second.clone(), LevelShape::OVERWORLD);
        let _ = reader.tables.set(Arc::new(SaveTables::new(&assets)));
        let loaded = reader
            .load_proto(sx, sz, assets.registry, &biomes)
            .expect("saved decorated proto chunk");
        assert!(loaded.decorated, "{name}: chests saved with features");
        assert_eq!(
            loaded.block_entities, center_entities,
            "{name}: chests, traps and loot tables survive the save"
        );
        assert_eq!(
            raw_start(&loaded, name),
            placed_start,
            "{name}: the placed start survives the save"
        );
        for &(table, count) in loot {
            assert_eq!(
                entities(&loaded.block_entities, table).len(),
                count,
                "{name}: {table} kept by the reader"
            );
        }
        // And the next worker starts from the same saved state: generating
        // the saved chunk again restores its block entities rather than
        // placing new loot.
        let reopened = Arc::new(VanillaTerrain::new(resources, 1234).expect("terrain"));
        let generator = Persistent {
            inner: reopened.clone(),
            save: Arc::new(reader),
            registry: assets.clone(),
            biomes: biomes.clone(),
        };
        let mut worker = generator.worker();
        let restored = generator.proto(&mut worker, sx, sz);
        let restored = reopened
            .saved_proto(&restored)
            .expect("restored proto chunk");
        assert_eq!(
            restored.block_entities, center_entities,
            "{name}: a restored chunk keeps its chests, traps and loot seeds"
        );
        assert_eq!(
            typed_start(&restored, name),
            placed_start,
            "{name}: a restored chunk keeps its start"
        );
        let _ = std::fs::remove_dir_all(&first);
        let _ = std::fs::remove_dir_all(&second);
    }
}

#[test]
fn chunk_attachments_survive_save_and_load() {
    use horizon_mod_runtime::attachments::{AttachmentCodec, AttachmentError, AttachmentType};
    let Some(root) = std::env::var_os("HORIZON_TEST_RESOURCES") else {
        return;
    };
    let root = PathBuf::from(root);
    let assets = crate::assets::Assets::load(&root).expect("assets");
    let biomes = Biomes::load(&horizon_assets::resource_source::ResourceRoot::directory(
        root.to_path_buf(),
    ))
    .expect("biomes");
    let key = AttachmentType {
        namespace: "test",
        name: "counter",
        default: || 0,
        codec: Some(AttachmentCodec {
            encode: |n: &i32| Ok([("count".into(), Tag::Int(*n))].into()),
            decode: |c: &Compound| match c.get(&"count".into()) {
                Some(Tag::Int(n)) => Ok(*n),
                _ => Err(AttachmentError::Decode),
            },
        }),
        copy_on_death: false,
        sync: false,
    };
    let mut chunk = Chunk::empty(2, 3);
    let mut extras = ChunkExtras::default();
    key.set(
        &mut extras.attachments,
        horizon_mod_runtime::attachments::Holder::Level,
        42,
    )
    .unwrap();
    let mut block_data = horizon_mod_runtime::attachments::Attachments::default();
    key.set(
        &mut block_data,
        horizon_mod_runtime::attachments::Holder::Level,
        21,
    )
    .unwrap();
    let pos = (33, 64, 50);
    let chest = assets.registry.default_state("minecraft:chest").unwrap();
    chunk.set(1, 64, 2, chest.0);
    chunk
        .block_entities
        .push((pos, crate::containers::BlockEntity::Chest(vec![None; 27])));
    extras.block_entity_attachments.push((pos, block_data));
    chunk.saved = Some(Arc::new(extras));
    let saved = encode(&chunk, assets.registry, &biomes);
    let mut loaded = decode(&saved, assets.registry, &biomes).expect("chunk");
    assert_eq!(
        key.get(
            &mut Arc::make_mut(loaded.saved.as_mut().unwrap()).attachments,
            horizon_mod_runtime::attachments::Holder::Level
        ),
        Ok(42)
    );
    let extras = Arc::make_mut(loaded.saved.as_mut().unwrap());
    assert_eq!(extras.block_entity_attachments[0].0, pos);
    assert_eq!(
        key.get(
            &mut extras.block_entity_attachments[0].1,
            horizon_mod_runtime::attachments::Holder::Level
        ),
        Ok(21)
    );
}

#[test]
fn unopened_pending_containers_save_their_loot_table() {
    let Some(root) = std::env::var_os("HORIZON_TEST_RESOURCES") else {
        return;
    };
    let root = PathBuf::from(root);
    let assets = crate::assets::Assets::load(&root).expect("assets");
    let biomes = Biomes::load(&horizon_assets::resource_source::ResourceRoot::directory(
        root.to_path_buf(),
    ))
    .expect("biomes");
    let tables = SaveTables::new(&assets);
    let chest = assets.registry.default_state("minecraft:chest").unwrap();
    let mut slots = vec![None; 27];
    slots[3] = Some(crate::items::ItemStack::new("minecraft:bone", 2));
    let mut chunk = Chunk::empty(0, 0);
    chunk.set(1, 64, 2, chest.0);
    chunk.block_entities.push((
        (1, 64, 2),
        crate::containers::BlockEntity::Chest(slots.clone()),
    ));
    let pending = PendingContainerLoot {
        table: string("minecraft:chests/simple_dungeon"),
        seed: Some(Tag::Long(42)),
    };
    chunk.block_entities[0].1.slots_mut().fill(None);
    chunk.saved = Some(Arc::new(ChunkExtras {
        pending_loot: [((1, 64, 2), pending)].into(),
        ..ChunkExtras::default()
    }));
    let saved_chest = |chunk: &Chunk| -> Tag {
        let saved = encode_with(
            chunk,
            assets.registry,
            &biomes,
            Some(&tables),
            LevelShape::OVERWORLD,
        );
        let Some(Tag::List(list)) = saved.tag.get("block_entities") else {
            panic!("no block entities");
        };
        list[0].clone()
    };
    let unopened = saved_chest(&chunk);
    assert_eq!(
        text(unopened.get("LootTable")).as_deref(),
        Some("minecraft:chests/simple_dungeon")
    );
    assert_eq!(number(unopened.get("LootTableSeed")), Some(42));
    assert!(unopened.get("Items").is_none());
    Arc::make_mut(chunk.saved.as_mut().unwrap())
        .pending_loot
        .remove(&(1, 64, 2));
    // The shared unpack transition consumes metadata before inventory changes.
    if let crate::containers::BlockEntity::Chest(s) = &mut chunk.block_entities[0].1 {
        s[3] = None;
        s[4] = Some(crate::items::ItemStack::new("minecraft:bone", 1));
    }
    let opened = saved_chest(&chunk);
    assert!(opened.get("LootTable").is_none());
    assert!(opened.get("Items").is_some());
    // Loaded with its table, rolled, then emptied: no table, or it refills on load.
    let mut extras = ChunkExtras::clone(chunk.saved.as_ref().unwrap());
    extras.block_entity_sources = vec![(
        (1, 64, 2),
        Tag::compound([
            ("id", string("minecraft:chest")),
            ("x", Tag::Int(1)),
            ("y", Tag::Int(64)),
            ("z", Tag::Int(2)),
            ("LootTable", string("minecraft:chests/simple_dungeon")),
            ("LootTableSeed", Tag::Long(42)),
        ]),
    )];
    chunk.saved = Some(Arc::new(extras));
    if let crate::containers::BlockEntity::Chest(s) = &mut chunk.block_entities[0].1 {
        s.fill(None);
    }
    let emptied = saved_chest(&chunk);
    assert!(emptied.get("LootTable").is_none());
    assert!(emptied.get("LootTableSeed").is_none());
}

#[test]
fn a_tall_level_saves_and_loads_its_whole_height() {
    let Some(root) = std::env::var_os("HORIZON_TEST_RESOURCES") else {
        return;
    };
    let root = PathBuf::from(root);
    let assets = crate::assets::Assets::load(&root).expect("assets");
    let biomes = Biomes::load(&horizon_assets::resource_source::ResourceRoot::directory(
        root.to_path_buf(),
    ))
    .expect("biomes");
    let tables = SaveTables::new(&assets);
    let stone = assets.registry.default_state("minecraft:stone").unwrap();
    // To The Max's Overworld: -64..2032, 131 sections.
    let shape = LevelShape::new(-64, 2096, 2096, true, false).unwrap();
    let mut chunk = Chunk::with_layout(0, 0, shape.column());
    chunk.set(0, -64, 0, stone.0);
    chunk.set(0, 2000, 0, stone.0);
    let saved = encode_with(&chunk, assets.registry, &biomes, Some(&tables), shape);
    assert_eq!(number(saved.tag.get("yPos")), Some(-4));
    let Some(Tag::List(sections)) = saved.tag.get("sections") else {
        panic!("no sections");
    };
    assert_eq!(sections.len(), 131);
    // 12-bit heightmap entries: y 2000's top reads 2001 - -64.
    let Some(Tag::LongArray(surface)) = saved
        .tag
        .get("Heightmaps")
        .and_then(|h| h.get("WORLD_SURFACE"))
    else {
        panic!("no WORLD_SURFACE");
    };
    assert_eq!(surface[0] & 0xfff, 2065);
    let back = decode_with(
        &saved,
        assets.registry,
        &biomes,
        Some(&tables),
        shape.column(),
    )
    .expect("decodes");
    assert_eq!(back.sections.len(), 131);
    assert_eq!(back.get(0, 2000, 0), stone.0);
    assert_eq!(back.get(0, -64, 0), stone.0);
    // Read into the vanilla column, the sections above it are left out.
    let short = decode_with(
        &saved,
        assets.registry,
        &biomes,
        Some(&tables),
        ColumnLayout::VANILLA,
    )
    .expect("decodes");
    assert_eq!(short.sections.len(), SECTIONS);
    assert_eq!(short.get(0, 2000, 0), 0);
    assert_eq!(short.get(0, -64, 0), stone.0);
}

/// `BlockEntity.getPosFromTag`: a block entity saved under another chunk's
/// coordinates is read into the chunk that saved it (vanilla warns and
/// keeps the low four bits), instead of landing where that chunk has no
/// block.
#[test]
fn a_block_entity_saved_under_another_chunks_coordinates_moves_here() {
    let Some(root) = std::env::var_os("HORIZON_TEST_RESOURCES") else {
        return;
    };
    let root = PathBuf::from(root);
    let assets = crate::assets::Assets::load(&root).expect("assets");
    let biomes = Biomes::load(&horizon_assets::resource_source::ResourceRoot::directory(
        root.clone(),
    ))
    .expect("biomes");
    // The destination must contain a chest: the fork validates saved block
    // entity membership before loading, independently of position repair.
    // The current saved block-state codec uses `id`, not the old `Name` key.
    let tag = horizon_core::nbt::parse_snbt(
            r#"{xPos:0,zPos:0,yPos:-4,Status:"minecraft:full",sections:[{Y:4b,block_states:{palette:[{id:"minecraft:chest"}]}}],block_entities:[{id:"minecraft:chest",x:37,y:70,z:-3,Items:[]}]}"#,
        )
        .unwrap()
        .0;
    let chunk = decode_with(
        &NamedTag {
            name: "".into(),
            tag,
        },
        assets.registry,
        &biomes,
        None,
        ColumnLayout::VANILLA,
    )
    .expect("chunk");
    let positions: Vec<(i32, i32, i32)> = chunk.block_entities.iter().map(|(p, _)| *p).collect();
    assert_eq!(
        positions,
        [(5, 70, 13)],
        "moved into the chunk that saved it"
    );
    let order = &chunk.saved.as_ref().expect("extras").block_entity_order;
    assert_eq!(
        order,
        &[(5, 70, 13)],
        "load order follows the moved position"
    );
}

#[test]
fn nether_and_end_chunks_save_with_their_own_height() {
    let Some(root) = std::env::var_os("HORIZON_TEST_RESOURCES") else {
        return;
    };
    let root = PathBuf::from(root);
    let assets = crate::assets::Assets::load(&root).expect("assets");
    let biomes = Biomes::load(&horizon_assets::resource_source::ResourceRoot::directory(
        root.to_path_buf(),
    ))
    .expect("biomes");
    let tables = SaveTables::new(&assets);
    let rack = assets
        .registry
        .default_state("minecraft:netherrack")
        .unwrap();
    let mut chunk = Chunk::empty(0, 0);
    for y in 0..128 {
        chunk.set(0, y, 0, rack.0);
    }
    let saved = encode_with(
        &chunk,
        assets.registry,
        &biomes,
        Some(&tables),
        LevelShape::NETHER,
    );
    assert_eq!(number(saved.tag.get("yPos")), Some(0));
    let Some(Tag::List(sections)) = saved.tag.get("sections") else {
        panic!("no sections");
    };
    let ys: Vec<i32> = sections
        .iter()
        .filter_map(|s| number(s.get("Y")).map(|y| y as i32))
        .collect();
    assert_eq!(ys, (0..16).collect::<Vec<_>>());
    let Some(Tag::List(post)) = saved.tag.get("PostProcessing") else {
        panic!("no PostProcessing");
    };
    assert_eq!(post.len(), 16);
    // Heightmap values count from the level's minimum: the column's top
    // block at y=127 reads 128 (vanilla), not 192 (from the Overworld's -64).
    let Some(Tag::LongArray(surface)) = saved
        .tag
        .get("Heightmaps")
        .and_then(|h| h.get("WORLD_SURFACE"))
    else {
        panic!("no WORLD_SURFACE");
    };
    assert_eq!(surface[0] & 0x1ff, 128);
    let overworld = encode_with(
        &chunk,
        assets.registry,
        &biomes,
        Some(&tables),
        LevelShape::OVERWORLD,
    );
    assert_eq!(number(overworld.tag.get("yPos")), Some(-4));
}

/// The far ring's reads (`lan::far`): a chunk saved with its light, without the gameplay's region
/// cache; none for a chunk not saved, or saved without its light.
#[test]
fn far_reads_take_saved_lit_chunks_only() {
    let Some(root) = std::env::var_os("HORIZON_TEST_RESOURCES") else {
        return;
    };
    let root = PathBuf::from(root);
    let assets = crate::assets::Assets::load(&root).expect("assets");
    let biomes = Biomes::load(&horizon_assets::resource_source::ResourceRoot::directory(
        root.to_path_buf(),
    ))
    .expect("biomes");
    let stone = assets.registry.default_state("minecraft:stone").unwrap();
    let mut lit = Chunk::empty(4, -7);
    lit.set(1, 70, 2, stone.0);
    let mut dark = lit.clone();
    dark.x = 5;
    lit.lit = true;
    let dir = std::env::temp_dir().join(format!("horizon-far-save-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let save = SaveDir::new(dir.clone(), LevelShape::OVERWORLD);
    let records = [&lit, &dark].map(|c| ((c.x, c.z), Some(encode(c, assets.registry, &biomes))));
    save.write_records("region", records.into(), true).unwrap();
    let reopened = SaveDir::new(dir.clone(), LevelShape::OVERWORLD);
    let far = reopened
        .load_far(4, -7, assets.registry, &biomes)
        .expect("saved with its light");
    assert_eq!(far.block(1, 70, 2), stone);
    assert!(far.lit && far.light_loaded);
    assert!(reopened.load_far(5, -7, assets.registry, &biomes).is_none());
    assert!(reopened.load_far(6, -7, assets.registry, &biomes).is_none());
    assert!(reopened.regions.lock().unwrap().is_empty());
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn saved_far_surface_reads_builds_and_water_without_gameplay_chunks() {
    let Some(root) = std::env::var_os("HORIZON_TEST_RESOURCES") else {
        return;
    };
    let root = PathBuf::from(root);
    let assets = crate::assets::Assets::load(&root).expect("assets");
    let biomes = Biomes::load(&horizon_assets::resource_source::ResourceRoot::directory(
        root.to_path_buf(),
    ))
    .expect("biomes");
    let stone = assets.registry.default_state("minecraft:stone").unwrap();
    let water = assets.registry.default_state("minecraft:water").unwrap();
    let sand = assets.registry.default_state("minecraft:sand").unwrap();
    let dirt = assets.registry.default_state("minecraft:dirt").unwrap();
    let leaves = assets
        .registry
        .default_state("minecraft:oak_leaves")
        .unwrap();
    let end_stone = assets
        .registry
        .default_state("minecraft:end_stone")
        .unwrap();
    let chorus = assets
        .registry
        .default_state("minecraft:chorus_plant")
        .unwrap();
    let flower = assets
        .registry
        .default_state("minecraft:chorus_flower")
        .unwrap();
    let mut chunk = Chunk::empty(-2, 3);
    chunk.set(0, 109, 0, stone.0);
    chunk.set(3, 121, 5, stone.0);
    chunk.set(3, 122, 5, stone.0);
    chunk.set(3, 120, 5, dirt.0);
    chunk.set(3, 119, 5, dirt.0);
    chunk.set(3, 115, 5, stone.0);
    chunk.set(11, 125, 11, stone.0);
    chunk.set(14, 127, 14, stone.0);
    chunk.set(14, 112, 14, stone.0);
    for y in 95..98 {
        chunk.set(2, y, 6, stone.0);
    }
    chunk.lit = true;
    let mut cave_light = Box::new([0x03; 4096]);
    cave_light[crate::chunk::index(12, 125, 11)] = 0xa7;
    Arc::make_mut(&mut chunk.sections[((121 - MIN_Y) >> 4) as usize]).light =
        Light::Full(cave_light);
    chunk.set(7, 90, 9, sand.0);
    chunk.set(7, 98, 9, leaves.0);
    let forest = biomes.id("minecraft:forest");
    chunk.set_biome_cell(7 / 4, (98 - MIN_Y) / 4, 9 / 4, forest);
    chunk.set(4, 54, 4, sand.0);
    for y in 55..64 {
        chunk.set(4, y, 4, water.0);
    }
    // Chorus is thin and non-solid but still part of the far End.
    chunk.set(12, 100, 2, end_stone.0);
    chunk.set(12, 101, 2, chorus.0);
    chunk.set(12, 102, 2, flower.0);
    let mut saved = encode(&chunk, assets.registry, &biomes);
    put(
        &mut saved.tag,
        "block_entities",
        Tag::String("not decoded by far sampling".into()),
    );
    let dir = std::env::temp_dir().join(format!("horizon-lod-save-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let save = SaveDir::new(dir.clone(), LevelShape::OVERWORLD);
    save.write_records("region", vec![((-2, 3), Some(saved))], true)
        .unwrap();
    let reopened = SaveDir::new(dir.clone(), LevelShape::OVERWORLD);
    let surfaces = reopened
        .lod_surfaces(-2, 3, &assets, &biomes)
        .expect("saved summary");
    assert_eq!(surfaces[0].surface.unwrap().height, 110);
    assert_eq!(surfaces[0].surface.unwrap().state, stone);
    let narrow_build = surfaces[5 * 16 + 3].surface.unwrap();
    assert_eq!((narrow_build.height, narrow_build.state), (123, stone));
    assert!(
        surfaces[5 * 16 + 2].surface.is_none(),
        "adjacent columns stay distinct"
    );
    let strata = &surfaces[5 * 16 + 3].spans;
    assert_eq!(
        strata
            .iter()
            .map(|span| (span.min, span.max, span.state))
            .collect::<Vec<_>>(),
        vec![(115, 116, stone), (119, 121, dirt), (121, 123, stone)]
    );
    assert!(
        strata.iter().all(|span| span.light == 0x3000),
        "saved cave darkness and block light survive"
    );
    assert_eq!(
        surfaces[11 * 16 + 11].spans[0].light,
        0x70a0,
        "neighbor face light uses separate sky/block channels"
    );
    let boundaries = &surfaces[14 * 16 + 14].spans;
    assert_eq!(boundaries.len(), 2);
    assert!(
        boundaries.iter().all(|span| span.light == 0x30f0),
        "face light crosses section boundaries above and below"
    );
    let across_sections = &surfaces[6 * 16 + 2].spans;
    assert_eq!(across_sections.len(), 1);
    assert_eq!((across_sections[0].min, across_sections[0].max), (95, 98));
    let canopy = surfaces[9 * 16 + 7].surface.unwrap();
    assert_eq!(
        (canopy.height, canopy.state, canopy.biome),
        (99, leaves, forest)
    );
    let underwater = surfaces[4 * 16 + 4].surface.unwrap();
    assert_eq!((underwater.height, underwater.state), (55, sand));
    assert_eq!(underwater.water, Some((64, water)));
    let chorus_column = &surfaces[2 * 16 + 12];
    assert_eq!(
        chorus_column
            .surface
            .map(|surface| (surface.height, surface.state)),
        Some((103, flower))
    );
    assert_eq!(
        chorus_column
            .spans
            .iter()
            .map(|span| (span.min, span.max, span.state))
            .collect::<Vec<_>>(),
        vec![
            (100, 101, end_stone),
            (101, 102, chorus),
            (102, 103, flower)
        ]
    );
    assert!(surfaces[15].surface.is_none(), "saved void stays empty");
    assert!(
        reopened.regions.lock().unwrap().is_empty(),
        "far reads do not grow gameplay region cache"
    );
    let mut proto = encode(&chunk, assets.registry, &biomes);
    put(
        &mut proto.tag,
        "Status",
        string(ChunkStatus::Terrain.name()),
    );
    put(&mut proto.tag, "xPos", Tag::Int(-3));
    save.write_records("region", vec![((-3, 3), Some(proto))], true)
        .unwrap();
    assert!(
        reopened.lod_surfaces(-3, 3, &assets, &biomes).is_none(),
        "unfinished proto saves must not suppress complete decoration"
    );
    let nether = SaveDir::new(dir.clone(), LevelShape::NETHER);
    let dark = nether.lod_surfaces(-2, 3, &assets, &biomes).unwrap();
    assert!(
        dark.iter()
            .flat_map(|c| &c.spans)
            .all(|s| s.light & 0xf0 == 0),
        "no invented skylight in Nether saves"
    );
    struct SavedOnly(Vec<crate::streaming::LodDetail>);
    impl crate::streaming::LodSampler for SavedOnly {
        fn sample(&mut self, _: i32, _: i32) -> Option<crate::lod::Surface> {
            panic!("saved data must win")
        }
        fn request_detail(&mut self, chunks: &[(i32, i32)], _: [i32; 2]) {
            assert!(
                chunks.is_empty(),
                "saved builds must not request speculative generation"
            );
        }
        fn drain_details(&mut self) -> Vec<crate::streaming::LodDetail> {
            std::mem::take(&mut self.0)
        }
    }
    let inner = Arc::new(crate::testgen::TestTerrain::new(assets.registry, &biomes).unwrap());
    let generator = Arc::new(Persistent {
        inner,
        save: Arc::new(reopened),
        registry: Arc::new(assets),
        biomes: Arc::new(biomes),
    });
    let mut sampler = SavedLodSampler {
        generator,
        inner: Box::new(SavedOnly(Vec::new())),
        cache: Default::default(),
        details: Vec::new(),
        approximate: false,
        generated: Arc::new(crate::lod::cache::DiskCache::new(dir.join("far"), 1)),
        bake: None,
    };
    use crate::streaming::LodSampler;
    sampler.request_detail(&[(-2, 3)], [-32, 48]);
    let details = sampler.drain_details();
    assert_eq!(details.len(), 1);
    assert_eq!(&details[0].1[..], &surfaces[..]);
    sampler.cache.clear();
    let generated: LodSurfaces = Arc::new(std::array::from_fn(|_| crate::lod::Column::default()));
    sampler.inner = Box::new(SavedOnly(vec![
        ((-2, 3), generated.clone()),
        ((-9, 9), generated.clone()),
    ]));
    let details = sampler.drain_details();
    assert_eq!(
        &details[0].1[..],
        &surfaces[..],
        "unrequested batch neighbours must preserve saved builds"
    );
    assert!(Arc::ptr_eq(&details[1].1, &generated));
    assert!(
        sampler.generated.get(-2, 3).is_none(),
        "saved builds are not cached as generated"
    );
    assert_eq!(sampler.generated.get(-9, 9).unwrap(), generated);
    // A later session reads the generated chunk back instead of
    // generating it again.
    sampler.request_detail(&[(-9, 9)], [-144, 144]);
    assert_eq!(sampler.drain_details()[0].1, generated);
    sampler.inner = crate::streaming::ChunkGenerator::lod_sampler(sampler.generator.inner.clone());
    let grid = sampler.grid(-32, 48, 2, 16);
    for z in 0..8 {
        for x in 0..8 {
            assert_eq!(
                grid[z * 16 + x],
                crate::lod::Column::reduced(&surfaces, x * 2, z * 2, 2),
                "saved builds must override a batched direct grid"
            );
        }
    }
    drop((save, sampler, nether));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn saved_far_surface_rejects_invalid_packed_palettes() {
    let mut states = Tag::compound([
        ("palette", Tag::List(vec![Tag::Byte(0), Tag::Byte(1)])),
        ("data", Tag::LongArray(vec![0; 256])),
    ]);
    let mut indices = [9u16; 4096];
    assert_eq!(lod_palette_indices(&states, &mut indices, 4), Some(2));
    assert_eq!(indices, [0; 4096]);
    // Two palette entries take 4 bits a block, 16 blocks a long: block 17
    // is the second of the second long, block 4095 the last of the last.
    let mut data = vec![0i64; 256];
    data[1] = 0x10;
    data[255] = 1 << 60;
    put(&mut states, "data", Tag::LongArray(data));
    assert_eq!(lod_palette_indices(&states, &mut indices, 4), Some(2));
    let set: Vec<usize> = (0..4096).filter(|&i| indices[i] != 0).collect();
    assert_eq!(set, [17, 4095]);
    assert_eq!((indices[17], indices[4095]), (1, 1));
    put(&mut states, "data", Tag::LongArray(vec![0; 255]));
    assert_eq!(lod_palette_indices(&states, &mut indices, 4), None);
    put(&mut states, "palette", Tag::List(Vec::new()));
    assert_eq!(lod_palette_indices(&states, &mut indices, 4), None);
    // One entry needs no data; the biomes' 64 cells pack from 1 bit.
    let one = Tag::compound([("palette", Tag::List(vec![Tag::Byte(0)]))]);
    let mut cells = [9u16; 64];
    assert_eq!(lod_palette_indices(&one, &mut cells, 1), Some(1));
    assert_eq!(cells, [0; 64]);
    let three = Tag::compound([
        ("palette", Tag::List(vec![Tag::Byte(0); 3])),
        // 2 bits a cell, 32 cells a long: cell 1 is 2, cell 33 is 1.
        ("data", Tag::LongArray(vec![0b1000, 0b0100])),
    ]);
    assert_eq!(lod_palette_indices(&three, &mut cells, 1), Some(3));
    assert_eq!((cells[0], cells[1], cells[33], cells[63]), (0, 2, 1, 0));
}

#[test]
#[ignore = "daily"]
fn saved_far_surface_checks_packed_indices_against_their_palettes() {
    let Some(root) = std::env::var_os("HORIZON_TEST_RESOURCES") else {
        return;
    };
    let root = PathBuf::from(root);
    let assets = crate::assets::Assets::load(&root).expect("assets");
    let biomes = Biomes::load(&horizon_assets::resource_source::ResourceRoot::directory(
        root.clone(),
    ))
    .expect("biomes");
    let stone = assets.registry.default_state("minecraft:stone").unwrap();
    let forest = biomes.id("minecraft:forest");
    let desert = biomes.id("minecraft:desert");
    assert!(forest != 0 && desert != 0 && forest != desert);
    // One section, y 0..16. Blocks: air and stone, 4 bits a block.
    // Biomes: three of them, 2 bits a 4x4x4 cell, 32 cells a long.
    let chunk = |blocks: Vec<i64>, cells: i64| NamedTag {
        name: "".into(),
        tag: Tag::compound([(
            "sections",
            Tag::List(vec![Tag::compound([
                ("Y", Tag::Byte(0)),
                (
                    "block_states",
                    Tag::compound([
                        (
                            "palette",
                            Tag::List(vec![
                                Tag::String("minecraft:air".into()),
                                Tag::String("minecraft:stone".into()),
                            ]),
                        ),
                        ("data", Tag::LongArray(blocks)),
                    ]),
                ),
                (
                    "biomes",
                    Tag::compound([
                        (
                            "palette",
                            Tag::List(vec![
                                Tag::String("minecraft:forest".into()),
                                Tag::String("minecraft:desert".into()),
                                Tag::String("minecraft:plains".into()),
                            ]),
                        ),
                        ("data", Tag::LongArray(vec![cells, 0])),
                    ]),
                ),
            ])]),
        )]),
    };
    let decode = |root: &NamedTag| {
        decode_lod_surfaces(root, &assets, &biomes, LevelShape::OVERWORLD.column(), true)
    };
    // Stone in the bottom layer (the first 256 blocks, 16 a long).
    let mut blocks = vec![0i64; 256];
    blocks[..16].fill(0x1111_1111_1111_1111);
    // The first cell (x 0..4) is desert, the second (x 4..8) names a
    // fourth biome the palette does not have, the others are forest.
    let columns = decode(&chunk(blocks.clone(), 0b1101)).expect("a valid section");
    let surface = |x: usize| columns[x].surface.unwrap();
    assert_eq!((surface(0).height, surface(0).state), (1, stone));
    assert_eq!(surface(0).biome, desert);
    assert_eq!(surface(4).biome, 0, "an index past the palette is no biome");
    assert_eq!(surface(8).biome, forest);
    // The last block names a third state the palette does not have.
    blocks[255] = 2 << 60;
    assert!(
        decode(&chunk(blocks, 0b1101)).is_none(),
        "a block index past the palette makes no far columns"
    );
}

/// #6386: far terrain looked a section's palette and data up by name for
/// each of its 4096 blocks, and its biomes again for each block it kept:
/// milliseconds for one saved chunk, most of them over the air above the
/// ground, on every far sampling thread.
#[test]
#[ignore = "daily"]
fn saved_far_surface_reads_a_section_once_not_once_a_block() {
    let Some(root) = std::env::var_os("HORIZON_TEST_RESOURCES") else {
        return;
    };
    let root = PathBuf::from(root);
    let assets = crate::assets::Assets::load(&root).expect("assets");
    let biomes = Biomes::load(&horizon_assets::resource_source::ResourceRoot::directory(
        root.clone(),
    ))
    .expect("biomes");
    let dandelion = assets
        .registry
        .default_state("minecraft:dandelion")
        .unwrap();
    // A flower in each section and air around it: nothing far terrain
    // keeps, so what is left to pay is reading the sections' blocks, as
    // loading the chunk for play does.
    let mut chunk = Chunk::empty(3, -5);
    for section in 0..chunk.sections.len() as i32 {
        chunk.set(8, MIN_Y + section * 16 + 8, 8, dandelion.0);
    }
    let saved = encode(&chunk, assets.registry, &biomes);
    let layout = LevelShape::OVERWORLD.column();
    let far = || decode_lod_surfaces(&saved, &assets, &biomes, layout, true);
    let columns = far().expect("far columns");
    assert!(
        columns
            .iter()
            .all(|column| column.surface.is_none() && column.spans.is_empty()),
        "a flower is no far terrain"
    );
    // The fastest of several runs of each, taken in turns, so that a busy
    // machine does not decide.
    let time = |run: &dyn Fn()| {
        let started = std::time::Instant::now();
        run();
        started.elapsed()
    };
    let (mut far_time, mut load_time) = (std::time::Duration::MAX, std::time::Duration::MAX);
    for _ in 0..25 {
        far_time = far_time.min(time(&|| {
            std::hint::black_box(far());
        }));
        load_time = load_time.min(time(&|| {
            std::hint::black_box(decode_with(&saved, assets.registry, &biomes, None, layout));
        }));
    }
    // Measured, optimized and not: 3 to 8 loads (the light layers come on
    // top of the blocks), and 100 to 300 with a lookup by name for each
    // block.
    assert!(
        far_time <= load_time * 32,
        "the far columns of a chunk took {far_time:?}, loading it {load_time:?}"
    );
}

#[test]
fn save_batch_preserves_order_coordinates_and_ownership() {
    use crate::containers::BlockEntity;
    let mut entities = HashMap::new();
    let mut names = HashMap::new();
    for x in [i32::MIN, -17, -16, -1, 0, 15, 16, i32::MAX] {
        for z in [-17, -16, -1, 0, 15, 16] {
            let p = (x, 64, z);
            entities.insert(p, BlockEntity::Chest(vec![None; 27]));
            names.insert(p, string(&format!("container {x},{z}")));
        }
    }
    // Names without a modelled entity must also survive the snapshot.
    names.insert((-2, 80, -3), string("orphan name"));
    let mut source = Chunk::empty(-1, -1);
    source
        .block_entities
        .push(((99, 0, 99), BlockEntity::Chest(vec![])));
    source.block_names.push(((99, 0, 99), string("stale")));
    source.saved = Some(Arc::new(ChunkExtras::default()));
    let source = Arc::new(source);
    let batch = vec![
        source.clone(),
        Arc::new(Chunk::empty(1, 1)),
        Arc::new(Chunk::empty(-2, 0)),
        source.clone(), // Preserve input order and repeated coordinates.
        Arc::new(Chunk::empty(i32::MIN >> 4, -2)),
        Arc::new(Chunk::empty(i32::MAX >> 4, 1)),
        Arc::new(Chunk::empty(100, 100)),
    ];
    let encode = |entries: &[((i32, i32, i32), BlockEntity)]| {
        entries
            .iter()
            .map(|(p, e)| block_entity_encode::encode(*p, e, "minecraft:chest"))
            .collect::<Vec<_>>()
    };
    for input in [&batch[..0], &batch[..1], &batch[..]] {
        let actual = chunks_with_block_entities(input, &entities, &names);
        assert_eq!(actual.len(), input.len());
        for (c, original) in actual.iter().zip(input) {
            let expected: Vec<_> = entities
                .iter()
                .filter(|(p, _)| (p.0 >> 4, p.2 >> 4) == (c.x, c.z))
                .map(|(p, e)| (*p, e.clone()))
                .collect();
            let expected_names: Vec<_> = names
                .iter()
                .filter(|(p, _)| (p.0 >> 4, p.2 >> 4) == (c.x, c.z))
                .map(|(p, n)| (*p, n.clone()))
                .collect();
            assert_eq!((c.x, c.z), (original.x, original.z));
            assert_eq!(encode(&c.block_entities), encode(&expected));
            assert_eq!(c.block_names, expected_names);
            assert!(Arc::ptr_eq(&c.sections[0], &original.sections[0]));
            if let Some(saved) = &original.saved {
                assert!(Arc::ptr_eq(c.saved.as_ref().unwrap(), saved));
            }
        }
    }
    let snapshot = chunks_with_block_entities(&batch, &entities, &names);
    entities.clear();
    names.clear();
    assert!(!snapshot[0].block_entities.is_empty());
    assert!(!snapshot[0].block_names.is_empty());
    assert_eq!(source.block_names[0].1, string("stale"));
    assert_eq!(source.block_entities[0].0, (99, 0, 99));
}

#[test]
fn work_shared_out_is_all_done_in_order_when_no_helper_can_start() {
    use super::share_out;
    let squares = |i: usize| i * i;
    let want: Vec<usize> = (0..40).map(squares).collect();
    // Every helper refused (a container at its thread limit): this thread does it all.
    assert_eq!(share_out(40, 4, squares, || false), want);
    // Some refused, some started.
    let mut asked = 0;
    let some = || {
        asked += 1;
        asked % 2 == 0
    };
    assert_eq!(share_out(40, 4, squares, some), want);
    assert_eq!(share_out(40, 4, squares, || true), want);
    assert_eq!(share_out(0, 4, squares, || true), Vec::<usize>::new());
}

/// `RegionFile.getVersion` writes a chunk with the version
/// `region-file-compression` selected: `compress_records` asks
/// `Compression::selected()` for every record it encodes, so a server told
/// to write lz4, gzip or uncompressed regions does
/// (`RegionFileVersion.configure`).
#[test]
fn a_chunk_is_written_with_the_selected_region_version() {
    use super::{Record, compress_records};
    let tag = Arc::new(NamedTag {
        name: "".into(),
        tag: Tag::compound([("xPos", Tag::Int(3))]),
    });
    let records = [((3, 4), Record::Tag(Arc::clone(&tag)))];
    let refs: Vec<_> = records.iter().map(|(k, r)| (*k, r)).collect();
    // Every other reader of the setting waits until the default is back.
    let _codec = super::compression_pool::SELECTED_CODEC
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for name in ["none", "lz4", "gzip", "deflate"] {
        assert!(Compression::configure(name), "{name}");
        let written = compress_records(&refs, 99)
            .pop()
            .expect("one record")
            .expect("an encoded record")
            .expect("a chunk, not a deletion");
        assert_eq!(
            written.compression,
            Compression::selected(),
            "written with {name}"
        );
        // The bytes are that codec's, and read back as the chunk.
        assert_eq!(written.nbt(Limits::default()).unwrap(), *tag);
    }
    // Leave the default behind for the rest of this process's tests.
    assert!(Compression::configure("deflate"));
}

#[test]
fn records_compressed_on_helpers_match_one_thread_in_order() {
    use super::{Record, compress_records};
    let records: Vec<((i32, i32), Record)> = (0..53i32)
        .map(|i| {
            let record = if i % 7 == 3 {
                Record::Delete
            } else {
                let payload: Vec<i64> = (0..i * 97 % 2000).map(|v| i64::from(v * i)).collect();
                Record::Tag(Arc::new(NamedTag {
                    name: "".into(),
                    tag: Tag::compound([("xPos", Tag::Int(i)), ("Data", Tag::LongArray(payload))]),
                }))
            };
            ((i, -i), record)
        })
        .collect();
    let refs: Vec<_> = records.iter().map(|(k, r)| (*k, r)).collect();
    let _codec = super::compression_pool::holding_selected_codec();
    let compressed = compress_records(&refs, 1234);
    assert_eq!(compressed.len(), records.len());
    for ((_, record), got) in records.iter().zip(compressed) {
        let want = record
            .tag()
            .map(|t| ChunkRecord::from_nbt(&t, Compression::selected(), 1234, Limits::default()))
            .transpose()
            .unwrap();
        let got = got.unwrap();
        assert_eq!(
            got.as_ref().map(|r| (&r.compressed, r.timestamp)),
            want.as_ref().map(|r| (&r.compressed, r.timestamp))
        );
    }
}

#[test]
fn older_worlds_are_upgraded_on_opening() {
    let dir = std::env::temp_dir().join(format!("horizon-dv-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let level = |v: i32| Tag::compound([("Data", Tag::compound([("DataVersion", Tag::Int(v))]))]);
    let version = |dir: &std::path::Path| {
        super::read_dat(&dir.join("level.dat"))
            .and_then(|r| super::number(r.get("Data")?.get("DataVersion")))
    };
    // 1.12.2 and 1.16.5: upgraded too, every server fix is ported. Data from before 1.13
    // needs the data fixers' tables of the prepared resources: without them (a checkout
    // nothing was prepared in, CI's verify) only 1.16.5 is upgraded here.
    let olds: &[i32] = if horizon_world::datafix::tables_available() {
        &[1343, 2586]
    } else {
        &[2586]
    };
    for &old in olds {
        super::write_dat(&dir.join("level.dat"), level(old)).unwrap();
        super::upgrade_older_world(&dir).unwrap();
        assert_eq!(version(&dir), Some(i64::from(super::DATA_VERSION)));
    }
    // 1.21.1: upgraded to the current version.
    super::write_dat(&dir.join("level.dat"), level(3955)).unwrap();
    super::upgrade_older_world(&dir).unwrap();
    assert_eq!(version(&dir), Some(i64::from(super::DATA_VERSION)));
    super::upgrade_older_world(&dir).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}
#[test]
fn region_files_kept_open_stay_under_vanillas_cache_size() {
    let open_fds = || std::fs::read_dir("/dev/fd").map_or(0, |d| d.count());
    let dir = std::env::temp_dir().join(format!("horizon-save-fds-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let save = super::SaveDir::new(dir.clone(), LevelShape::OVERWORLD);
    let before = open_fds();
    let held = super::cached_region(
        &save.regions,
        (0, 0),
        &save.region_path("region", 0, 0),
        true,
    )
    .unwrap()
    .unwrap();
    // A region whose write a failed sync left staged, held by no one, is
    // not closed for room either: the next write finishes that commit.
    let staged = super::cached_region(
        &save.regions,
        (-1, 0),
        &save.region_path("region", -1, 0),
        true,
    )
    .unwrap()
    .unwrap();
    staged
        .lock()
        .unwrap()
        .write(
            ChunkPos::new(-32, 0),
            &horizon_world::region::ChunkRecord {
                compression: horizon_world::compression::Compression::None,
                compressed: vec![1; 10],
                timestamp: 1,
            },
        )
        .unwrap();
    let staged = {
        let weak = Arc::downgrade(&staged);
        drop(staged);
        weak
    };
    for x in 1..=600 {
        super::cached_region(
            &save.regions,
            (x, 0),
            &save.region_path("region", x, 0),
            true,
        )
        .unwrap()
        .unwrap();
    }
    // Other tests open files at the same time: a margin, far below 600.
    let grown = open_fds().saturating_sub(before);
    assert!(
        grown <= super::MAX_OPEN_REGIONS + 32,
        "{grown} descriptors more"
    );
    let cache = save.regions.lock().unwrap();
    assert!(
        cache.open() <= super::MAX_OPEN_REGIONS,
        "{} open",
        cache.open()
    );
    // The one still held is never closed under its holder.
    assert!(Arc::ptr_eq(
        &cache.regions[&(0, 0)].0.clone().unwrap(),
        &held
    ));
    let kept = cache.regions[&(-1, 0)].0.clone().unwrap();
    assert!(std::ptr::eq(Arc::as_ptr(&kept), staged.as_ptr()));
    assert!(kept.lock().unwrap().has_staged_changes());
    drop(kept);
    drop(cache);
    // What was written to regions since closed reads back from the disk.
    let entity = |x: i32| Tag::compound([("x", Tag::Int(x))]);
    save.save_entities_later((0..300).map(|i| ((i * 32, 0), vec![entity(i)])).collect())
        .unwrap();
    save.flush().unwrap();
    assert!(save.entity_regions.lock().unwrap().open() <= super::MAX_OPEN_REGIONS);
    for i in [0, 1, 150, 299] {
        assert_eq!(save.load_entities(i * 32, 0), vec![entity(i)]);
    }
    drop(held);
    let _ = std::fs::remove_dir_all(&dir);
}
#[test]
fn write_completion_polls_without_waiting_and_keeps_latest_snapshot() {
    let dir = std::env::temp_dir().join(format!("horizon-save-poll-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let save = super::SaveDir::new(dir.clone(), LevelShape::OVERWORLD);
    let lock = super::SessionLock::acquire(&dir).unwrap();
    let path = save.region_path("entities", 0, 0);
    let region = super::cached_region(&save.entity_regions, (0, 0), &path, true)
        .unwrap()
        .unwrap();
    // Hold the actual region lock, rather than relying on disk speed or sleeps.
    let blocked = region.lock().unwrap();
    let entity = |version| Tag::compound([("version", Tag::Int(version))]);
    save.save_entities_later(vec![((1, 2), vec![entity(1)])])
        .unwrap();
    save.save_entities_later(vec![((1, 2), vec![entity(2)])])
        .unwrap();
    assert!(!save.writes_finished().unwrap());
    assert_eq!(save.active_chunk_writes(), 0); // entity batches do not count
    save.write_records("region", vec![((0, 0), None), ((1, 0), None)], false)
        .unwrap();
    // The writer is blocked above: two chunk records in ONE queued batch.
    assert_eq!(save.active_chunk_writes(), 2);
    assert_eq!(save.load_entities(1, 2), vec![entity(2)]);
    assert!(super::SessionLock::acquire(&dir).is_err());
    drop(blocked);
    save.flush().unwrap();
    assert!(save.writes_finished().unwrap());
    assert_eq!(save.active_chunk_writes(), 0);
    let reopened = super::SaveDir::new(dir.clone(), LevelShape::OVERWORLD);
    assert_eq!(reopened.load_entities(1, 2), vec![entity(2)]);
    save.save_entities_later(vec![((1, 2), vec![])]).unwrap();
    save.flush().unwrap();
    assert!(
        super::SaveDir::new(dir.clone(), LevelShape::OVERWORLD)
            .load_entities(1, 2)
            .is_empty()
    );
    drop((reopened, save, lock));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn failed_async_writes_are_readable_and_never_reported_durable() {
    let dir = std::env::temp_dir().join(format!("horizon-save-failure-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // A file where a directory is needed causes a real writer I/O error.
    std::fs::write(dir.join("entities"), b"blocked").unwrap();
    let save = super::SaveDir::new(dir.clone(), LevelShape::OVERWORLD);
    let entity = Tag::compound([("version", Tag::Int(3))]);
    save.save_entities_later(vec![((0, 0), vec![entity.clone()])])
        .unwrap();
    assert!(save.flush().is_err());
    assert!(save.writes_finished().is_err());
    assert_eq!(save.load_entities(0, 0), vec![entity]);
    // The failure stays visible on subsequent barriers too.
    assert!(save.flush().is_err());
    drop(save);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn session_lock_is_exclusive() {
    let dir = std::env::temp_dir().join(format!("horizon-lock-{}", std::process::id()));
    let first = super::SessionLock::acquire(&dir).expect("first lock");
    assert!(super::SessionLock::acquire(&dir).is_err());
    drop(first);
    // Windows refuses a read through another handle while the file is
    // locked, so the bytes are read once the lock is released.
    assert_eq!(
        std::fs::read(dir.join("session.lock")).unwrap(),
        [0x26, 0x03]
    );
    assert!(super::SessionLock::acquire(&dir).is_ok());
    let _ = std::fs::remove_dir_all(&dir);
}
/// #749: a copy of the lock's descriptor (a child process spawned on
/// another thread holds one until it execs) must not keep a released
/// world locked.
#[test]
fn session_lock_releases_while_a_descriptor_copy_lives() {
    let dir = std::env::temp_dir().join(format!("horizon-lock-copy-{}", std::process::id()));
    let lock = super::SessionLock::acquire(&dir).expect("first lock");
    let copy = lock.0.try_clone().unwrap();
    drop(lock);
    assert!(super::SessionLock::acquire(&dir).is_ok());
    drop(copy);
    let _ = std::fs::remove_dir_all(&dir);
}
#[test]
fn crafter_block_entity_round_trips() {
    let mut crafter = crate::containers::Crafter::default();
    crafter.items[2] = Some(crate::items::ItemStack::new("minecraft:stick", 5));
    crafter.disabled[4] = true;
    crafter.triggered = 1;
    crafter.crafting_ticks = 3;
    let tag = block_entity_encode::encode(
        (7, 8, 9),
        &crate::containers::BlockEntity::Crafter(crafter),
        "minecraft:crafter",
    );
    let (pos, back) = decode_block_entity(&tag).expect("crafter");
    assert_eq!(pos, (7, 8, 9));
    let crate::containers::BlockEntity::Crafter(back) = back else {
        panic!("not a crafter");
    };
    assert_eq!(back.items[2].as_ref().map(|s| s.count), Some(5));
    assert!(back.disabled[4] && !back.disabled[3]);
    assert!(back.is_triggered());
    assert_eq!(back.crafting_ticks, 3);
    assert_eq!(back.redstone_signal(), 2);
}
/// `triggered` is an int vanilla saves as it is and calls triggered only at 1, so other
/// values come back unchanged and untriggered.
#[test]
fn crafter_triggered_keeps_the_whole_int() {
    for value in [0, 1, 5, -1, 65537] {
        let crafter = crate::containers::Crafter {
            triggered: value,
            ..Default::default()
        };
        let tag = block_entity_encode::encode(
            (0, 0, 0),
            &crate::containers::BlockEntity::Crafter(crafter),
            "minecraft:crafter",
        );
        let (_, back) = decode_block_entity(&tag).expect("crafter");
        let crate::containers::BlockEntity::Crafter(back) = back else {
            panic!("not a crafter");
        };
        assert_eq!(back.triggered, value);
        assert_eq!(back.is_triggered(), value == 1, "triggered {value}");
    }
}
#[test]
fn crafter_triggered_double_load_floors_and_saturates() {
    for (value, expected) in [(-0.5_f64, -1), (1.5, 1), (4294967297.0, i32::MAX)] {
        let tag = Tag::compound([
            ("id", string("minecraft:crafter")),
            ("x", Tag::Int(0)),
            ("y", Tag::Int(0)),
            ("z", Tag::Int(0)),
            ("triggered", Tag::Double(value.to_bits())),
        ]);
        let (_, back) = decode_block_entity(&tag).expect("crafter");
        let crate::containers::BlockEntity::Crafter(back) = back else {
            panic!("not a crafter");
        };
        assert_eq!(back.triggered, expected, "triggered {value}d");
        assert_eq!(back.is_triggered(), expected == 1, "triggered {value}d");
    }
}
#[test]
fn campfire_block_entity_round_trips() {
    let mut c = crate::containers::Campfire::default();
    c.items[1] = Some(crate::items::ItemStack::new("minecraft:beef", 1));
    c.progress[1] = 120;
    c.total[1] = 600;
    let tag = block_entity_encode::encode(
        (3, 64, -2),
        &crate::containers::BlockEntity::Campfire(Box::new(c.clone())),
        "minecraft:soul_campfire",
    );
    // One block entity type for both campfires.
    assert_eq!(text(tag.get("id")).as_deref(), Some("minecraft:campfire"));
    let (pos, back) = decode_block_entity(&tag).expect("campfire");
    assert_eq!(pos, (3, 64, -2));
    let crate::containers::BlockEntity::Campfire(back) = back else {
        panic!("not a campfire");
    };
    assert_eq!(*back, c);
    // The update tag a server's clients get: the items alone.
    let campfire = crate::containers::BlockEntity::Campfire(Box::new(c));
    let update = crate::lan::chunks::entity_tag(&campfire, false).expect("update tag");
    assert!(update.get("Items").is_some() && update.get("CookingTimes").is_none());
}
/// `LecternBlockEntity.loadAdditional`: `Page` is clamped to
/// `0..getPageCount(book) - 1`, so a stored page outside the book loads as
/// the first or last page the block holds. A bookless lectern counts no
/// pages and loads page -1.
#[test]
fn a_lecterns_page_loads_within_its_book() {
    let page_of = |tag: &str| {
        let tag = horizon_core::nbt::parse_snbt(tag).unwrap().0;
        let (_, entity) = decode_block_entity(&tag).expect("lectern");
        let crate::containers::BlockEntity::Lectern(l) = entity else {
            panic!("not a lectern");
        };
        l.page
    };
    let book = |pages: &str| {
        format!(
            r#"id:"minecraft:written_book",count:1,components:{{
                    "minecraft:written_book_content":{{title:"t",author:"a",pages:[{pages}]}}
                }}"#
        )
    };
    // Two pages: a stored 7 is the last page, a stored -3 the first.
    assert_eq!(
        page_of(&format!(
            r#"{{id:"minecraft:lectern",x:1,y:64,z:2,Page:7,Book:{{{}}}}}"#,
            book(r#""one","two""#)
        )),
        1
    );
    assert_eq!(
        page_of(&format!(
            r#"{{id:"minecraft:lectern",x:1,y:64,z:2,Page:-3,Book:{{{}}}}}"#,
            book(r#""one","two""#)
        )),
        0
    );
    // No book counts no pages, so any page loads as -1, as vanilla's.
    assert_eq!(
        page_of(r#"{id:"minecraft:lectern",x:1,y:64,z:2,Page:4}"#),
        -1
    );
    // A page already inside the book is kept, and saved back as it is.
    let tag = format!(
        r#"{{id:"minecraft:lectern",x:1,y:64,z:2,Page:1,Book:{{{}}}}}"#,
        book(r#""one","two","three""#)
    );
    let tag = horizon_core::nbt::parse_snbt(&tag).unwrap().0;
    let (pos, entity) = decode_block_entity(&tag).expect("lectern");
    let crate::containers::BlockEntity::Lectern(l) = entity else {
        panic!("not a lectern");
    };
    assert_eq!(l.page, 1);
    let saved = block_entity_encode::encode(
        pos,
        &crate::containers::BlockEntity::Lectern(l),
        "minecraft:lectern",
    );
    assert_eq!(number(saved.get("Page")), Some(1));
}
#[test]
fn a_chest_loot_table_takes_precedence_over_saved_items() {
    let mut saved = horizon_core::nbt::parse_snbt(
            r#"{id:"minecraft:chest",x:1,y:2,z:3,Items:[{Slot:3b,id:"minecraft:diamond",count:7}],LootTable:"minecraft:chests/simple_dungeon",LootTableSeed:42L}"#,
        )
        .unwrap()
        .0;
    let (_, chest) = decode_block_entity(&saved).expect("chest");
    assert!(
        chest.slots().iter().all(Option::is_none),
        "ChestBlockEntity.loadAdditional ignores Items when a loot table loads"
    );
    // The command path keeps an empty inventory and the pending table/seed
    // for data get and saving. No loot resources are needed to keep that state.
    let pending = PendingContainerLoot::from_tag(&saved).unwrap();
    let mut stored = block_entity_data((1, 2, 3), &chest, "minecraft:chest", None);
    let Tag::Compound(fields) = &mut stored else {
        panic!("compound");
    };
    pending.write_tags(fields);
    assert_eq!(stored.get("LootTable"), saved.get("LootTable"));
    assert_eq!(stored.get("LootTableSeed"), saved.get("LootTableSeed"));
    assert!(stored.get("Items").is_none());
    let Tag::Compound(fields) = &mut saved else {
        panic!("compound");
    };
    fields.remove(&"LootTable".into());
    let (_, chest) = decode_block_entity(&saved).expect("chest without table");
    let item = chest.slots()[3].as_ref().expect("saved item");
    assert_eq!(item.item.as_ref(), "minecraft:diamond");
    assert_eq!(item.count, 7);
}

/// `tryLoadLootTable` in every `RandomizableContainer`'s `loadAdditional`
/// (barrel, shulker boxes, hopper, dispenser, dropper):
/// a valid table leaves the saved `Items` unread, as for the chest.
#[test]
fn every_randomizable_container_lets_its_loot_table_win_over_saved_items() {
    for (id, size) in [
        ("minecraft:barrel", 27),
        ("minecraft:shulker_box", 27),
        ("minecraft:white_shulker_box", 27),
        ("minecraft:hopper", crate::containers::HOPPER_SLOTS),
        ("minecraft:dispenser", 9),
        ("minecraft:dropper", 9),
    ] {
        let mut saved = horizon_core::nbt::parse_snbt(&format!(
                r#"{{id:"{id}",x:1,y:2,z:3,Items:[{{Slot:3b,id:"minecraft:diamond",count:7}}],LootTable:"minecraft:chests/simple_dungeon",LootTableSeed:42L}}"#
            ))
            .unwrap()
            .0;
        let (_, container) = decode_block_entity(&saved).expect(id);
        assert_eq!(container.slots().len(), size, "{id}");
        assert!(
            container.slots().iter().all(Option::is_none),
            "{id}: loadAdditional ignores Items when a loot table loads"
        );
        let pending = PendingContainerLoot::from_tag(&saved).unwrap();
        let mut stored = block_entity_data((1, 2, 3), &container, id, None);
        let Tag::Compound(fields) = &mut stored else {
            panic!("compound");
        };
        pending.write_tags(fields);
        assert_eq!(stored.get("LootTable"), saved.get("LootTable"), "{id}");
        assert_eq!(
            stored.get("LootTableSeed"),
            saved.get("LootTableSeed"),
            "{id}"
        );
        assert!(stored.get("Items").is_none(), "{id}");
        let Tag::Compound(fields) = &mut saved else {
            panic!("compound");
        };
        fields.remove(&"LootTable".into());
        let (_, container) = decode_block_entity(&saved).expect(id);
        let item = container.slots()[3].as_ref().expect("saved item");
        assert_eq!(item.item.as_ref(), "minecraft:diamond", "{id}");
        assert_eq!(item.count, 7, "{id}");
    }
}

#[test]
fn invalid_container_loot_tables_leave_saved_items_intact() {
    for id in [
        "minecraft:chest",
        "minecraft:trapped_chest",
        "minecraft:barrel",
        "minecraft:shulker_box",
        "minecraft:hopper",
        "minecraft:dispenser",
        "minecraft:dropper",
    ] {
        for invalid in ["INVALID TABLE", "bad:table/with spaces", "bad@id"] {
            let saved = horizon_core::nbt::parse_snbt(&format!(
                    r#"{{id:"{id}",x:1,y:2,z:3,Items:[{{Slot:0b,id:"minecraft:diamond",count:3}}],LootTable:"{invalid}"}}"#
                ))
                .unwrap()
                .0;
            let (_, container) = decode_block_entity(&saved).expect(id);
            let item = container.slots()[0]
                .as_ref()
                .expect("saved item with invalid table");
            assert_eq!(item.item.as_ref(), "minecraft:diamond");
            assert_eq!(item.count, 3);
        }
    }
}

/// `LockCode`: a chest's `lock` is not in Horizon's model, so `data get`
/// takes it from the tag the block entity was loaded (or set) with.
#[test]
fn a_locked_chest_keeps_its_lock_through_decode_and_encode() {
    let saved = horizon_core::nbt::parse_snbt(
        r#"{id:"minecraft:chest",x:1,y:2,z:3,lock:{items:"minecraft:gold_ingot"}}"#,
    )
    .unwrap()
    .0;
    let (pos, chest) = decode_block_entity(&saved).expect("chest");
    assert_eq!(chest.slots().len(), 27);
    assert_eq!(pos, (1, 2, 3));
    // The encoded entity has no field to carry the lock...
    let plain = block_entity_encode::encode(pos, &chest, "minecraft:chest");
    assert!(plain.get("lock").is_none());
    // ...so `data get` puts back the one it was loaded with.
    let data = block_entity_data(pos, &chest, "minecraft:chest", saved.get("lock"));
    let lock = horizon_core::nbt::parse_snbt(r#"{items:"minecraft:gold_ingot"}"#)
        .unwrap()
        .0;
    assert_eq!(data.get("lock"), Some(&lock));
}
/// A loot roll's block entity keeps the `components` and `lock` a model
/// does not hold, merged the way a chunk save merges them.
#[test]
fn a_loot_chest_keeps_the_components_and_lock_it_was_loaded_with() {
    let saved = horizon_core::nbt::parse_snbt(
        r#"{id:"minecraft:chest",x:1,y:2,z:3,CustomName:"Loot",lock:{items:"minecraft:gold_ingot"},components:{"minecraft:custom_data":{k:1}}}"#,
    )
    .unwrap()
    .0;
    let (pos, chest) = decode_block_entity(&saved).expect("chest");
    let name = Tag::String("Loot".into());
    let full = full_block_entity_data(
        pos,
        &chest,
        "minecraft:chest",
        Some(&saved),
        Some(&name),
        saved.get("lock"),
    );
    assert_eq!(full.get("components"), saved.get("components"));
    assert_eq!(full.get("lock"), saved.get("lock"));
    assert_eq!(full.get("CustomName"), Some(&name));
    assert!(full.get("keepPacked").is_none());
}
#[test]
fn trial_spawner_save_overlay_uses_normalized_inline_configs() {
    let saved = horizon_core::nbt::parse_snbt(
        r#"{id:"minecraft:trial_spawner",x:1,y:2,z:3,normal_config:{spawn_range:4,total_mobs:6.0f},ominous_config:{spawn_range:5,total_mobs:6.0f},components:{"minecraft:custom_data":{k:1}}}"#,
    )
    .unwrap()
    .0;
    let (pos, spawner) = decode_block_entity(&saved).expect("trial_spawner");
    let full = full_block_entity_data(
        pos,
        &spawner,
        "minecraft:trial_spawner",
        Some(&saved),
        None,
        None,
    );
    assert_eq!(full.get("normal_config"), None);
    assert_eq!(
        full.get("ominous_config"),
        Some(&horizon_core::nbt::parse_snbt("{spawn_range:5}").unwrap().0)
    );
    assert_eq!(full.get("components"), saved.get("components"));
}
#[test]
fn spawner_block_entity_round_trips() {
    // A dungeon spawner as vanilla saves it (`id` is mob_spawner, not the block).
    let saved = horizon_core::nbt::parse_snbt(
            r#"{id:"minecraft:mob_spawner",x:67,y:-52,z:-64,Delay:20s,SpawnData:{entity:{id:"minecraft:zombie"}},SpawnPotentials:[]}"#,
        )
        .unwrap()
        .0;
    let (pos, back) = decode_block_entity(&saved).expect("spawner");
    assert_eq!(pos, (67, -52, -64));
    let crate::containers::BlockEntity::Spawner(spawner) = &back else {
        panic!("not a spawner");
    };
    let next = spawner.next_data().and_then(|d| d.id());
    assert_eq!(next.as_deref(), Some("minecraft:zombie"));
    let tag = block_entity_encode::encode(pos, &back, "minecraft:spawner");
    let (_, again) = decode_block_entity(&tag).expect("spawner again");
    let crate::containers::BlockEntity::Spawner(again) = again else {
        panic!("not a spawner");
    };
    assert_eq!(again.update_tag(), spawner.update_tag());
}
#[test]
#[ignore = "daily"]
fn native_saved_ids_must_match_the_declared_block_membership() {
    // The production registry is immutable. Exercise its real readers in a fresh process.
    const CHILD: &str = "HORIZON_TEST_NATIVE_SAVED_MEMBERSHIP";
    if std::env::var_os(CHILD).is_none() {
        for mode in ["accepted", "revoked"] {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--include-ignored",
                    "--exact",
                    "save::tests::native_saved_ids_must_match_the_declared_block_membership",
                    "--nocapture",
                ])
                .env(CHILD, mode)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "test=save::tests::native_saved_ids_must_match_the_declared_block_membership exit_code={:?} category=fixture-child-failed",
                output.status.code()
            );
        }
        return;
    }
    use crate::block_entity_types::{self, BlockEntityTypes};
    let mut types = BlockEntityTypes::new(
        [
            ("test:press".into(), "test:machine".into()),
            ("test:mill".into(), "test:machine".into()),
            ("test:board".into(), "test:other".into()),
        ]
        .into(),
        vec![
            "test:machine".into(),
            "test:other".into(),
            "minecraft:chest".into(),
        ],
    );
    // Seed the same canonical pairs as production when resources are available.
    // The explicit shared-representation cases below also run without resources.
    let mut vanilla_blocks: Vec<String> = [
        "chest",
        "trapped_chest",
        "barrel",
        "dropper",
        "dispenser",
        "hopper",
        "shulker_box",
    ]
    .into_iter()
    .map(|name| format!("minecraft:{name}"))
    .collect();
    for color in [
        "white",
        "orange",
        "magenta",
        "light_blue",
        "yellow",
        "lime",
        "pink",
        "gray",
        "light_gray",
        "cyan",
        "purple",
        "blue",
        "brown",
        "green",
        "red",
        "black",
    ] {
        vanilla_blocks.push(format!("minecraft:{color}_shulker_box"));
    }
    if let Some(root) = std::env::var_os("HORIZON_TEST_RESOURCES") {
        let path = horizon_protocol::registry::Builtin::find(&PathBuf::from(root))
            .expect("registries.json");
        let builtin = horizon_protocol::registry::Builtin::load(&path).unwrap();
        vanilla_blocks.extend(
            builtin
                .get("minecraft:block")
                .unwrap()
                .names
                .iter()
                .cloned(),
        );
    }
    for block in vanilla_blocks {
        if let Some(kind) = crate::lan::chunks::block_entity_type(&block) {
            types.template(block.clone(), block, kind.into());
        }
    }
    types.template(
        "minecraft:trapped_chest".into(),
        "minecraft:trapped_chest".into(),
        "minecraft:trapped_chest".into(),
    );
    let accepted = std::env::var(CHILD).unwrap() == "accepted";
    if accepted {
        types
            .add_valid("minecraft:trapped_chest", "minecraft:chest")
            .unwrap();
    }
    block_entity_types::install(types).unwrap();
    let tag = |id: &str| {
        horizon_core::nbt::parse_snbt(&format!(r#"{{id:"{id}",x:1,y:64,z:2,energy:900L}}"#,))
            .unwrap()
            .0
    };
    for (block, kind) in block_entity_types::current().unwrap().memberships() {
        if block.starts_with("minecraft:") {
            let saved = tag(kind);
            // Some vanilla types have no client representation yet. Membership
            // must not discard any entity that the decoder can represent.
            assert_eq!(
                decode_block_entity_for_block(&saved, block).is_some(),
                decode_block_entity(&saved).is_some(),
                "{kind} on {block}"
            );
        }
    }
    assert_eq!(
        decode_block_entity_for_block(&tag("minecraft:chest"), "minecraft:trapped_chest").is_some(),
        accepted
    );
    assert!(decode_block_entity_for_block(&tag("minecraft:banner"), "minecraft:stone").is_none());
    assert!(
        decode_block_entity_for_block(&tag("minecraft:banner"), "minecraft:trapped_chest")
            .is_none()
    );
    for block in [
        "trapped_chest",
        "barrel",
        "dropper",
        "dispenser",
        "hopper",
        "shulker_box",
        "red_shulker_box",
    ] {
        let block = format!("minecraft:{block}");
        let kind = crate::lan::chunks::block_entity_type(&block).unwrap();
        assert!(
            decode_block_entity_for_block(&tag(kind), &block).is_some(),
            "{kind} on {block}"
        );
    }
    for block in ["test:press", "test:mill"] {
        let (_, entity) = decode_block_entity_for_block(&tag("test:machine"), block).unwrap();
        assert!(entity.fits(block));
        assert_eq!(entity.id(), "test:machine");
        let crate::containers::BlockEntity::Mod(m) = entity else {
            panic!("not native");
        };
        assert_eq!(m.nbt.get(&"energy".into()), Some(&Tag::Long(900)));
        for id in [
            "test:press",
            "test:other",
            "test:unknown",
            "minecraft:banner",
        ] {
            assert!(
                decode_block_entity_for_block(&tag(id), block).is_none(),
                "{id}"
            );
        }
    }
    for block in [
        "test:casing",
        "test:board",
        "other:press",
        "minecraft:stone",
    ] {
        assert!(decode_block_entity_for_block(&tag("test:machine"), block).is_none());
    }
    assert!(
        decode_block_entity_for_block(&tag("minecraft:banner"), "minecraft:white_banner").is_some()
    );
}

#[test]
fn queued_client_update_fields_are_not_saved_or_taken_by_a_save() {
    let saved = horizon_core::nbt::parse_snbt(r#"{id:"test:machine",x:5,y:-60,z:12,progress:7}"#)
        .unwrap()
        .0;
    let (pos, entity) = decode_block_entity(&saved).unwrap();
    let crate::containers::BlockEntity::Mod(machine) = &entity else {
        panic!("not a mod's");
    };
    let batch = Compound::from([("effect".into(), Tag::Int(23))]);
    machine.set_client_update_fields(batch.clone());
    assert_eq!(
        block_entity_encode::encode(pos, &entity, "test:machine"),
        saved
    );
    assert_eq!(machine.take_client_update_fields(), Some(batch));
}

#[test]
fn mod_block_entity_round_trips() {
    let saved = horizon_core::nbt::parse_snbt(
            r#"{id:"gtmod:macerator",x:5,y:-60,z:12,energy:1200L,progress:7,Items:[{Slot:0b,id:"minecraft:iron_ore",count:3}]}"#,
        )
        .unwrap()
        .0;
    let (pos, back) = decode_block_entity(&saved).expect("mod block entity");
    assert_eq!(pos, (5, -60, 12));
    let crate::containers::BlockEntity::Mod(m) = &back else {
        panic!("not a mod's");
    };
    assert_eq!(m.id(), "gtmod:macerator");
    // The mod sees its own fields only.
    for key in ["id", "x", "y", "z"] {
        assert!(!m.nbt.contains_key(&key.into()), "{key}");
    }
    assert_eq!(m.nbt.get(&"energy".into()), Some(&Tag::Long(1200)));
    // `Items` are its slots, not the mod's compound.
    assert!(!m.nbt.contains_key(&"Items".into()));
    assert_eq!(m.items.len(), 1);
    assert_eq!(m.items[0].as_ref().map(|s| s.count), Some(3));
    assert_eq!(
        block_entity_encode::encode(pos, &back, "gtmod:macerator"),
        saved
    );
    // `Items` that are not an item list stay the mod's, as saved.
    for items in [
        r#"Items:{energy:5}"#,
        r#"Items:[1,2,3]"#,
        r#"Items:[{Slot:0b,id:"minecraft:air",count:1}]"#,
        r#"Items:[{Slot:0b,id:"minecraft:stone",count:1,note:"mine"}]"#,
    ] {
        let saved =
            horizon_core::nbt::parse_snbt(&format!(r#"{{id:"gtmod:cable",x:0,y:0,z:0,{items}}}"#))
                .unwrap()
                .0;
        let (pos, back) = decode_block_entity(&saved).expect("mod block entity");
        let crate::containers::BlockEntity::Mod(m) = &back else {
            panic!("not a mod's");
        };
        assert!(m.items.is_empty(), "{items}");
        assert_eq!(
            block_entity_encode::encode(pos, &back, "gtmod:cable"),
            saved,
            "{items}"
        );
    }
    // Vanilla's types are never a mod's, modelled or not.
    let ender = horizon_core::nbt::parse_snbt(r#"{id:"minecraft:ender_chest",x:0,y:0,z:0}"#)
        .unwrap()
        .0;
    assert!(!matches!(
        decode_block_entity(&ender),
        Some((_, crate::containers::BlockEntity::Mod(_)))
    ));
}
/// `CopperGolemStatueBlockEntity.createStatue`: the golem's styled custom
/// name lives in the statue's components, through every weathering stage,
/// a save and its item.
#[test]
fn copper_golem_statue_keeps_the_golem_name() {
    use crate::containers::BlockEntity;
    let name = Tag::compound([
        ("text", string("Bob")),
        ("color", string("gold")),
        ("bold", Tag::Byte(1)),
    ]);
    let components: Compound = [("minecraft:custom_name".into(), name.clone())].into();
    for block in [
        "minecraft:copper_golem_statue",
        "minecraft:exposed_copper_golem_statue",
        "minecraft:weathered_copper_golem_statue",
        "minecraft:oxidized_copper_golem_statue",
        "minecraft:waxed_oxidized_copper_golem_statue",
    ] {
        // Scraping and waxing swap the block, so every stage keeps the entity.
        let entity = BlockEntity::for_block(block).expect(block);
        assert!(entity.fits(block), "{block}");
        let BlockEntity::CopperGolemStatue(_) = entity else {
            panic!("{block}: not a statue");
        };
    }
    let entity = BlockEntity::CopperGolemStatue(components.clone());
    let tag =
        block_entity_encode::encode((1, 2, 3), &entity, "minecraft:oxidized_copper_golem_statue");
    assert_eq!(
        tag.get("id"),
        Some(&string("minecraft:copper_golem_statue"))
    );
    let (pos, back) = decode_block_entity(&tag).expect("statue");
    assert_eq!(pos, (1, 2, 3));
    let BlockEntity::CopperGolemStatue(back) = back else {
        panic!("not a statue");
    };
    assert_eq!(back, components);
    // The statue item carries the name (`getItem`: collectComponents).
    let saved = crate::block_entity_nbt::canonical(&tag).expect("canonical statue");
    let patch = crate::block_entity_item::pick(&saved);
    assert_eq!(patch.get(&"minecraft:custom_name".into()), Some(&name));
    // Placing that item gives the entity the name back.
    let fresh = crate::block_entity_nbt::canonical(&block_entity_encode::encode(
        (1, 2, 3),
        &BlockEntity::CopperGolemStatue(Compound::new()),
        "minecraft:copper_golem_statue",
    ))
    .expect("canonical statue");
    let placed = crate::block_entity_item::place(&fresh, &patch, false);
    let (_, placed) = decode_block_entity(&placed).expect("placed statue");
    assert!(matches!(placed, BlockEntity::CopperGolemStatue(c) if c == components));
}
#[test]
fn banner_block_entity_round_trips() {
    let banner = crate::containers::Banner {
        layers: vec![crate::banner::BannerLayer::new("minecraft:creeper", "lime")],
        name: Some(string("Flag")),
        components: [("minecraft:rarity".into(), string("uncommon"))].into(),
    };
    let tag = block_entity_encode::encode(
        (1, 2, 3),
        &crate::containers::BlockEntity::Banner(banner.clone()),
        "minecraft:red_banner",
    );
    let (pos, back) = decode_block_entity(&tag).expect("banner");
    assert_eq!(pos, (1, 2, 3));
    let crate::containers::BlockEntity::Banner(back) = back else {
        panic!("not a banner");
    };
    assert_eq!(back.layers, banner.layers);
    assert_eq!(back.name, banner.name);
    assert_eq!(back.components, banner.components);
}
use super::*;
use crate::chunk::{MIN_Y, SECTIONS};

/// `TheEndGatewayBlockEntity.loadAdditional` reads `ExactTeleport` with
/// `getBooleanOr`: a number is true when its byte value is not 0, so an
/// int 256 is false and 257 true.
#[test]
fn an_end_gateways_exact_teleport_is_read_as_a_byte() {
    let exact =
        |saved: Tag| decode_end_gateway(&Tag::compound([("ExactTeleport", saved)])).exact_teleport;
    assert!(exact(Tag::Byte(1)));
    assert!(!exact(Tag::Byte(0)));
    assert!(!exact(Tag::Int(256)));
    assert!(exact(Tag::Int(257)));
    assert!(!decode_end_gateway(&Tag::compound([("Age", Tag::Long(1))])).exact_teleport);
}

#[test]
fn a_mods_fluid_ticks_are_saved_by_name() {
    use horizon_world::fluid::{FluidKind, ModFluidRules, ModFluidType, register_mod_fluid};
    let fluid = register_mod_fluid(ModFluidType {
        id: "savetest:oil".into(),
        flowing_id: "savetest:flowing_oil".into(),
        bucket: None,
        rules: ModFluidRules {
            tick_delay: 5,
            drop_off: 1,
            slope_find_distance: 4,
            converts_to_source: false,
        },
    });
    let fluid = fluid.unwrap();
    let tick = |kind: FluidKind| SavedTick {
        kind: horizon_core::ticks::TickTypeId(kind.tick_id()),
        pos: BlockPos::new(1, 64, 2),
        delay: 3,
        priority: horizon_core::ticks::TickPriority::Normal,
    };
    let ticks = [
        tick(FluidKind::ModFlowing(fluid)),
        tick(FluidKind::Mod(fluid)),
        tick(FluidKind::FlowingWater),
        tick(FluidKind::Lava),
    ];
    let tag = tick_list(&ticks, fluid_name);
    let names: Vec<_> = match &tag {
        Tag::List(list) => list.iter().filter_map(|t| text(t.get("i"))).collect(),
        _ => Vec::new(),
    };
    assert_eq!(
        names,
        [
            "savetest:flowing_oil",
            "savetest:oil",
            "minecraft:flowing_water",
            "minecraft:lava"
        ]
    );
    let back = read_ticks(Some(&tag), (0, 0), fluid_id);
    assert_eq!(
        back.iter().map(|t| t.kind).collect::<Vec<_>>(),
        ticks.iter().map(|t| t.kind).collect::<Vec<_>>()
    );
}

#[test]
fn only_server_disk_records_intern_saved_fluid_names() {
    // The game initializes the platform at startup; the unresolved-fluid table lives there,
    // and without it nothing is interned. Alone, nothing else in this process has done it.
    mod_platform::__private::initialize().unwrap();
    let root = std::env::temp_dir().join(format!("horizon-fluid-source-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let mut save = SaveDir::new(root.clone(), LevelShape::OVERWORLD);
    save.enable_trusted_fluid_load();
    let record = |id: &str| NamedTag {
        name: "".into(),
        tag: Tag::compound([(
            "block_entities",
            Tag::List(vec![Tag::compound([(
                "fluids",
                Tag::List(vec![Tag::compound([("Fluid", Tag::String(id.into()))])]),
            )])]),
        )]),
    };
    let disk = "fixture:trusted_saved_fluid";
    let live = "fixture:untrusted_pending_fluid";
    fs::create_dir_all(root.join("region")).unwrap();
    let mut region = RegionFile::open_writable(save.region_path("region", 0, 0), true).unwrap();
    region
        .write(
            ChunkPos::new(0, 0),
            &ChunkRecord::from_nbt(&record(disk), Compression::Zlib, 0, Limits::saved()).unwrap(),
        )
        .unwrap();
    drop(region);
    save.read_record_raw("region", 0, 0).unwrap();
    assert!(mod_api::__private::lookup_unresolved_fluid(disk).is_some());
    let oversized = format!("fixture:{}", "x".repeat(257));
    let original = record(&oversized);
    intern_saved_fluids(&original);
    assert!(mod_api::__private::lookup_unresolved_fluid(&oversized).is_none());
    assert_eq!(original, record(&oversized));
    save.pending.lock().unwrap().insert(
        (Folder::Region, 1, 0),
        (0, Record::Tag(Arc::new(record(live)))),
    );
    save.read_record_raw("region", 1, 0).unwrap();
    assert!(mod_api::__private::lookup_unresolved_fluid(live).is_none());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn end_gateway_block_entity_round_trips() {
    use crate::containers::{BlockEntity, EndGateway};
    let plain = BlockEntity::for_block("minecraft:end_gateway").expect("gateway");
    assert!(matches!(&plain, BlockEntity::EndGateway(g) if *g == EndGateway::default()));
    for exit in [None, Some([8, 100, 0])] {
        for exact in [false, true] {
            let gateway = EndGateway {
                age: 201,
                exit_portal: exit,
                exact_teleport: exact,
            };
            let tag = block_entity_encode::encode(
                (702, 100, 0),
                &BlockEntity::EndGateway(gateway.clone()),
                "minecraft:end_gateway",
            );
            assert_eq!(
                text(tag.get("id")).as_deref(),
                Some("minecraft:end_gateway")
            );
            assert_eq!(tag.get("Age"), Some(&Tag::Long(201)));
            assert_eq!(
                tag.get("exit_portal"),
                exit.map(|p| Tag::IntArray(p.to_vec())).as_ref()
            );
            assert_eq!(
                tag.get("ExactTeleport"),
                exact.then_some(Tag::Byte(1)).as_ref()
            );
            let (pos, back) = decode_block_entity(&tag).expect("gateway");
            assert_eq!(pos, (702, 100, 0));
            assert!(matches!(back, BlockEntity::EndGateway(g) if g == gateway));
        }
    }
}

#[test]
fn end_gateway_exit_requires_spawnable_bounds() {
    for (exit, valid) in [
        (vec![-30_000_000, -20_000_000, -30_000_000], true),
        (vec![29_999_999, 19_999_999, 29_999_999], true),
        (vec![30_000_000, 0, 0], false),
        (vec![0, 0, 30_000_000], false),
        (vec![-30_000_001, 0, 0], false),
        (vec![0, 0, -30_000_001], false),
        (vec![0, 20_000_000, 0], false),
        (vec![0, -20_000_001, 0], false),
        (vec![i32::MIN, 0, 0], false),
        (vec![0, 100], false),
    ] {
        let tag = Tag::compound([
            ("id", string("minecraft:end_gateway")),
            ("x", Tag::Int(702)),
            ("y", Tag::Int(100)),
            ("z", Tag::Int(0)),
            ("exit_portal", Tag::IntArray(exit)),
        ]);
        let (_, crate::containers::BlockEntity::EndGateway(g)) = decode_block_entity(&tag).unwrap()
        else {
            panic!("gateway");
        };
        assert_eq!(g.exit_portal.is_some(), valid);
        let canonical = crate::block_entity_nbt::canonical(&tag).expect("gateway NBT");
        assert_eq!(canonical.get("exit_portal").is_some(), valid);
    }
    assert_eq!(
        decode_end_gateway(&Tag::Compound(Compound::new())),
        crate::containers::EndGateway::default()
    );
}

#[test]
fn skull_block_entity_keeps_its_profile() {
    // A placed `player_head` as vanilla saves it: the owning player's
    // `profile` survives load, encode and `canonical`.
    let saved = horizon_core::nbt::parse_snbt(
            r#"{id:"minecraft:skull",x:10,y:64,z:20,profile:{name:"Steve",id:[I;1,2,3,4],properties:[{name:"textures",value:"abc"}]},note_block_sound:"minecraft:entity.player.hurt",custom_name:"Trophy"}"#,
        )
        .unwrap()
        .0;
    let (pos, back) = decode_block_entity(&saved).expect("skull");
    assert_eq!(pos, (10, 64, 20));
    let crate::containers::BlockEntity::Skull(skull) = &back else {
        panic!("not a skull");
    };
    assert_eq!(skull.profile, saved.get("profile").cloned());
    assert_eq!(
        skull.note_block_sound.as_deref(),
        Some("minecraft:entity.player.hurt")
    );
    assert_eq!(back.id(), "minecraft:skull");
    let tag = block_entity_encode::encode(pos, &back, "minecraft:player_head");
    assert_eq!(
        crate::block_entity_nbt::id(&tag).as_deref(),
        Some("minecraft:skull")
    );
    assert_eq!(tag.get("profile"), saved.get("profile"));
    let (_, again) = decode_block_entity(&tag).expect("skull again");
    let crate::containers::BlockEntity::Skull(again) = again else {
        panic!("not a skull");
    };
    assert_eq!(again.profile, skull.profile);
    // A profile-less head (skeleton, creeper) round-trips too.
    let plain = block_entity_encode::encode(
        pos,
        &crate::containers::BlockEntity::Skull(crate::containers::Skull::default()),
        "minecraft:creeper_head",
    );
    let (_, decoded) = decode_block_entity(&plain).expect("plain skull");
    assert!(matches!(decoded, crate::containers::BlockEntity::Skull(_)));
    // The heads and skulls have one; the piston's head is no skull.
    use crate::containers::BlockEntity;
    assert!(matches!(
        BlockEntity::for_block("minecraft:player_wall_head"),
        Some(BlockEntity::Skull(_))
    ));
    assert!(BlockEntity::for_block("minecraft:piston_head").is_none());
}

#[test]
fn uuid_text_round_trips() {
    let u = [-158327916, 1572358581, -2125825222, -487948715];
    assert_eq!(uuid_from_text(&uuid_text(u)), Some(u));
    assert_eq!(
        uuid_text([0, 1, 2, 3]),
        "00000000-0000-0001-0000-000200000003"
    );
    let r = random_uuid();
    assert_eq!((r[1] >> 12) & 0xF, 4, "version 4");
}
#[test]
fn random_uuids_made_at_once_differ() {
    let made: std::collections::HashSet<[i32; 4]> = (0..10_000).map(|_| random_uuid()).collect();
    assert_eq!(made.len(), 10_000);
}
#[test]
fn creaking_heart_keeps_its_protector_uuid() {
    use crate::containers::{BlockEntity, CreakingHeart};
    let heart = CreakingHeart {
        creaking: Some((Some(7), [1, 2, 3, 4])),
        ..Default::default()
    };
    let tag = block_entity_encode::encode(
        (3, 70, -5),
        &BlockEntity::CreakingHeart(Box::new(heart)),
        "minecraft:creaking_heart",
    );
    assert_eq!(
        tag.get("creaking"),
        Some(&Tag::IntArray(vec![1, 2, 3, 4])),
        "vanilla stores the protector's UUID under `creaking`"
    );
    let (pos, entity) = decode_block_entity(&tag).expect("heart decodes");
    assert_eq!(pos, (3, 70, -5));
    let BlockEntity::CreakingHeart(h) = entity else {
        panic!("a creaking heart");
    };
    // The entity id is resolved again once the creaking is loaded.
    assert_eq!(h.creaking, Some((None, [1, 2, 3, 4])));
    let lonely = block_entity_encode::encode(
        (0, 0, 0),
        &BlockEntity::CreakingHeart(Box::default()),
        "minecraft:creaking_heart",
    );
    assert!(lonely.get("creaking").is_none());
}
#[test]
fn light_nibbles_use_vanilla_order() {
    let mut a = Box::new([0u8; 4096]);
    a[0] = 0xF3; // sky 15, block 3 at x=0
    a[1] = 0x52; // sky 5, block 2 at x=1
    let light = Light::Full(a);
    assert_eq!(nibbles(&light, false)[0], 0x23);
    assert_eq!(nibbles(&light, true)[0], 0x5F);
}
/// The independent reference: parse the saved layers and let
/// `crate::light::light_from_layers` expand and repack them.
fn reference(tag: &Tag, chunk: &mut Chunk, sky_light: bool) {
    if tag.get("isLightOn") != Some(&Tag::Byte(1)) {
        return;
    }
    let Some(Tag::List(sections)) = tag.get("sections") else {
        return;
    };
    let mut layers: Vec<(Option<&[u8]>, Option<&[u8]>)> = vec![(None, None); SECTIONS];
    for section in sections {
        let Some(y) = number(section.get("Y")) else {
            continue;
        };
        let si = y as i32 - (MIN_Y >> 4);
        if !(0..SECTIONS as i32).contains(&si) {
            continue;
        }
        let layer = |name: &str| match section.get(name) {
            Some(Tag::ByteArray(a)) if a.len() == 2048 => Some(&a[..]),
            _ => None,
        };
        layers[si as usize] = (layer("SkyLight"), layer("BlockLight"));
    }
    let above = [if sky_light { 15 } else { 0 }; 256];
    crate::light::light_from_layers(chunk, &layers, above, sky_light);
}

#[test]
fn stored_light_loads_with_vanilla_defaults() {
    let mut sky = vec![0u8; 2048];
    sky[0] = 0x5A; // x=0 -> 10, x=1 -> 5
    let section = |y: i8, entries: Vec<(&str, Tag)>| {
        let mut all = vec![("Y", Tag::Byte(y))];
        all.extend(entries);
        Tag::compound(all)
    };
    let tag = Tag::compound([
        ("isLightOn", Tag::Byte(1)),
        (
            "sections",
            Tag::List(vec![
                section(
                    -4,
                    vec![
                        ("SkyLight", Tag::ByteArray(sky)),
                        ("BlockLight", Tag::ByteArray(vec![0x21; 2048])),
                    ],
                ),
                // No sky layer: repeats the bottom layer of the section above.
                section(-3, vec![]),
            ]),
        ),
    ]);
    let mut chunk = Chunk::empty(0, 0);
    load_light(&tag, &mut chunk, true);
    assert!(chunk.lit && chunk.light_loaded);
    let at = |c: &Chunk, si: usize, i: usize| c.sections[si].light(i);
    assert_eq!(at(&chunk, 0, 0), 0xA1, "stored sky 10, block 1");
    assert_eq!(at(&chunk, 0, 1), 0x52, "stored sky 5, block 2");
    // Section -3 has no layers; above it (-2..) sky defaults to 15.
    assert_eq!(at(&chunk, 1, 0), 0xF0);
    assert_eq!(at(&chunk, SECTIONS - 1, 5), 0xF0);
    let mut dark = Chunk::empty(0, 0);
    load_light(&tag, &mut dark, false);
    assert_eq!(at(&dark, 1, 0), 0x00, "no skylight dimension");
    let mut unlit = Chunk::empty(0, 0);
    load_light(
        &Tag::compound([("isLightOn", Tag::Byte(0))]),
        &mut unlit,
        true,
    );
    assert!(!unlit.lit && !unlit.light_loaded);
}
#[test]
#[ignore = "daily"]
fn saved_light_matches_reference_for_layers_and_metadata() {
    let section = |y: Tag, sky: Option<Tag>, block: Option<Tag>| {
        let mut entries = vec![("Y", y)];
        if let Some(v) = sky {
            entries.push(("SkyLight", v));
        }
        if let Some(v) = block {
            entries.push(("BlockLight", v));
        }
        Tag::compound(entries)
    };
    let root = |sections| {
        Tag::compound([
            ("isLightOn", Tag::Byte(1)),
            ("sections", Tag::List(sections)),
        ])
    };
    let check = |tag: &Tag| {
        for sky_light in [false, true] {
            let mut expected = Chunk::empty(-3, 5);
            let s = Arc::make_mut(&mut expected.sections[0]);
            s.light = Light::Full(Box::new([0xab; 4096]));
            s.blocks.set(0, 1);
            s.blocks.set(0, 0);
            s.biomes.set(0, 1);
            s.biomes.set(0, 0);
            let mut actual = expected.clone();
            reference(tag, &mut expected, sky_light);
            load_light(tag, &mut actual, sky_light);
            assert_eq!(
                (actual.lit, actual.light_loaded),
                (expected.lit, expected.light_loaded)
            );
            for (a, b) in actual.sections.iter().zip(&expected.sections) {
                assert_eq!(
                    std::mem::discriminant(&a.light),
                    std::mem::discriminant(&b.light)
                );
                assert_eq!(
                    format!("{:?}", a.blocks.to_wire()),
                    format!("{:?}", b.blocks.to_wire())
                );
                assert_eq!(
                    format!("{:?}", a.biomes.to_wire()),
                    format!("{:?}", b.biomes.to_wire())
                );
                for i in 0..4096 {
                    assert_eq!(a.light(i), b.light(i), "index {i}, sky {sky_light}");
                }
            }
        }
    };
    for tag in [
        Tag::compound(Vec::<(&str, Tag)>::new()),
        Tag::compound([("isLightOn", Tag::Byte(0))]),
        Tag::compound([("isLightOn", Tag::Byte(2))]),
        Tag::compound([("isLightOn", Tag::Int(1)), ("sections", Tag::List(vec![]))]),
        Tag::compound([("isLightOn", Tag::Byte(1)), ("sections", Tag::Int(0))]),
        root(vec![]),
    ] {
        check(&tag);
    }
    // Cover every repeated byte, including 0xF0/0x0F: a uniform byte
    // array need not hold uniform nibbles.
    for byte in 0..=255u8 {
        check(&root(vec![section(
            Tag::Byte(19),
            Some(Tag::ByteArray(vec![byte; 2048])),
            Some(Tag::ByteArray(vec![255 - byte; 2048])),
        )]));
    }
    // Every pair of uniform sky/block nibble values, with missing
    // sections below inheriting the stored bottom row.
    for sky in 0..16u8 {
        for block in 0..16u8 {
            check(&root(vec![section(
                Tag::Byte(1),
                Some(Tag::ByteArray(vec![sky * 17; 2048])),
                Some(Tag::ByteArray(vec![block * 17; 2048])),
            )]));
        }
    }
    for length in [0, 1, 2047, 2048, 2049] {
        for invalid in [
            Tag::Int(1),
            Tag::List(vec![]),
            Tag::ByteArray(vec![0xf0; length]),
        ] {
            check(&root(vec![section(
                Tag::Byte(-4),
                Some(invalid.clone()),
                Some(invalid),
            )]));
        }
    }
    let varied: Vec<u8> = (0..2048)
        .map(|i| (i as u8).wrapping_mul(73).wrapping_add(31))
        .collect();
    check(&root(vec![
        section(Tag::Byte(-5), Some(Tag::ByteArray(varied.clone())), None),
        section(Tag::Byte(20), Some(Tag::ByteArray(varied.clone())), None),
        section(
            Tag::String("19".into()),
            Some(Tag::ByteArray(varied.clone())),
            None,
        ),
        section(Tag::Byte(19), Some(Tag::ByteArray(varied.clone())), None),
        section(Tag::Byte(17), None, Some(Tag::ByteArray(varied.clone()))),
        section(Tag::Byte(-4), Some(Tag::ByteArray(varied.clone())), None),
        // Last duplicate wins, even when its arrays have wrong types.
        section(
            Tag::Byte(-4),
            Some(Tag::Int(15)),
            Some(Tag::ByteArray(vec![7; 2047])),
        ),
        section(Tag::Byte(0), None, Some(Tag::ByteArray(varied))),
    ]));
}
/// A saved sky layer is the resident `Light::Sky` layer byte for byte,
/// so a section whose block light is uniformly zero can adopt it.
#[test]
fn a_varying_sky_layer_is_adopted_without_expansion() {
    let saved: Vec<u8> = (0..2048).map(|i| (i as u8).wrapping_mul(29)).collect();
    let tag = Tag::compound([
        ("isLightOn", Tag::Byte(1)),
        (
            "sections",
            Tag::List(vec![Tag::compound([
                ("Y", Tag::Byte(3)),
                ("SkyLight", Tag::ByteArray(saved.clone())),
            ])]),
        ),
    ]);
    let mut chunk = Chunk::empty(0, 0);
    let mut expected = chunk.clone();
    load_light(&tag, &mut chunk, true);
    reference(&tag, &mut expected, true);
    for i in 0..4096 {
        assert_eq!(chunk.sections[7].light(i), expected.sections[7].light(i));
    }
    let Light::Sky(layer, 0) = &chunk.sections[7].light else {
        panic!("varying sky with no block light is Light::Sky(_, 0)");
    };
    assert_eq!(
        &layer[..],
        &saved[..],
        "resident layer equals the saved one"
    );
}
/// #2571: the difficulty is a property of the world: the level data
/// holds it, so a world that was set to hard is hard when it is opened
/// again, and a world that was downloaded keeps the difficulty it was
/// made with.
#[test]
fn the_level_data_holds_the_difficulty() {
    let dir = std::env::temp_dir().join(format!("horizon-difficulty-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let mut level = Level {
        name: "Test".into(),
        ..Level::default()
    };
    // The world's difficulty is the level data's.
    level.set_difficulty(3);
    assert_eq!(level.difficulty, "hard");
    assert_eq!(level.difficulty_id(), 3);
    // It survives the save, so the world comes back with it.
    level.write(&dir).unwrap();
    let back = Level::read(&dir).unwrap();
    assert_eq!(back.difficulty, "hard");
    assert_eq!(back.difficulty_id(), 3);
    // A world whose difficulty is not one of ours is normal.
    let other = Level {
        difficulty: "extreme".into(),
        ..Level::default()
    };
    assert_eq!(other.difficulty_id(), 2);
    let _ = fs::remove_dir_all(&dir);
}

/// Dedicated world saves must not overwrite separately managed player data.
#[test]
fn dedicated_world_save_preserves_player_file() {
    let dir = std::env::temp_dir().join(format!(
        "horizon-dedicated-player-save-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);

    let mut level = Level {
        name: "Test".into(),
        position: Some([12.5, 70.0, -8.25]),
        inventory: vec![(0, "minecraft:diamond".into(), 7, 0)],
        ..Level::default()
    };

    // Create a real player file using the existing singleplayer writer.
    level.write(&dir).unwrap();
    let uuid = level.player_uuid.unwrap();
    let player_file = dir
        .join("players")
        .join("data")
        .join(format!("{}.dat", uuid_text(uuid)));

    let before = fs::read(&player_file).unwrap();

    // Simulate an incomplete world-level player state.
    level.position = None;
    level.inventory.clear();
    level.day_time = 12345;

    // Dedicated saves must update metadata without touching player data.
    level.write_without_player(&dir).unwrap();

    let after = fs::read(&player_file).unwrap();
    assert_eq!(before, after, "dedicated save overwrote player data");

    // The world metadata must still be persisted.
    let reopened = Level::read(&dir).unwrap();
    assert_eq!(reopened.day_time, 12345);

    let _ = fs::remove_dir_all(&dir);
}

/// A crash's save before the first normal one keeps a legacy owner's only copy (#18117).
#[test]
fn a_crash_save_keeps_a_legacy_level_dat_player() {
    let dir = std::env::temp_dir().join(format!(
        "horizon-crash-legacy-player-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    let mut level = Level {
        name: "Test".into(),
        inventory: vec![(0, "minecraft:diamond".into(), 7, 0)],
        ..Level::default()
    };
    level.write(&dir).unwrap();
    // The owner's file moves into level.dat's Data.Player, as an older world keeps it.
    let player_file = dir
        .join("players")
        .join("data")
        .join(format!("{}.dat", uuid_text(level.player_uuid.unwrap())));
    let player = read_dat(&player_file).unwrap();
    fs::remove_file(&player_file).unwrap();
    let mut root = read_dat(&dir.join("level.dat")).unwrap();
    if let Some(data) = root.get_mut("Data") {
        put(data, "Player", player);
    }
    write_dat(&dir.join("level.dat"), root).unwrap();

    let mut crashed = Level::read(&dir).unwrap();
    crashed.inventory.clear();
    crashed.write_keeping_player(&dir).unwrap();

    let reopened = Level::read(&dir).unwrap();
    assert_eq!(
        reopened.inventory,
        vec![(0, "minecraft:diamond".to_owned(), 7, 0)]
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn the_ends_clock_keeps_its_own_time() {
    let dir = std::env::temp_dir().join(format!("horizon-end-clock-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let mut level = Level {
        name: "Test".into(),
        time: 5000,
        day_time: 9000,
        ..Level::default()
    };
    // A world without one starts the End's clock at its game time.
    level.write(&dir).unwrap();
    assert_eq!(Level::read(&dir).unwrap().end_clock, Some(5000));
    // Then it keeps its own count, apart from the game time and the day.
    level.time = 8000;
    level.end_clock = Some(321);
    level.write(&dir).unwrap();
    let back = Level::read(&dir).unwrap();
    assert_eq!(
        (back.time, back.day_time, back.end_clock),
        (8000, 9000, Some(321))
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn level_round_trips_through_the_vanilla_files() {
    let dir = std::env::temp_dir().join(format!("horizon-level-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let mut level = Level {
        name: "Test".into(),
        seed: -42,
        time: 1234,
        day_time: 25000,
        creative: true,
        position: Some([1.5, 70.0, -3.25]),
        rotation: [90.0, 10.0],
        health: 17.0,
        absorption: 5.0,
        food: 15,
        saturation: 2.5,
        inventory: vec![
            (0, "minecraft:stone".into(), 12, 0),
            (5, "minecraft:iron_sword".into(), 1, 3),
            (103, "minecraft:iron_helmet".into(), 1, 0),
        ],
        xp_level: 3,
        xp_progress: 0.5,
        xp_total: 30,
        dimension: "minecraft:the_nether".into(),
        world_spawn: Some([8, 64, 8]),
        world_spawn_angle: [45.0, -10.0],
        spawn: Some([20.0, 70.0, 20.0]),
        difficulty_locked: true,
        flying: true,
        ..Level::default()
    };
    level.weather.raining = true;
    level.weather.rain_time = 500;
    level
        .game_rules
        .insert("minecraft:keep_inventory".into(), 1);
    level.write(&dir).unwrap();
    let back = Level::read(&dir).unwrap();
    assert_eq!(back.name, "Test");
    assert_eq!(back.seed, -42);
    assert_eq!((back.time, back.day_time), (1234, 25000));
    assert!(back.creative);
    assert!(back.flying);
    assert_eq!(back.position, Some([1.5, 70.0, -3.25]));
    assert_eq!(back.rotation, [90.0, 10.0]);
    assert_eq!((back.health, back.food, back.saturation), (17.0, 15, 2.5));
    assert_eq!(back.absorption, 5.0);
    let mut inv = back.inventory.clone();
    inv.sort();
    assert_eq!(
        inv,
        vec![
            (0, "minecraft:stone".into(), 12, 0),
            (5, "minecraft:iron_sword".into(), 1, 3),
            (103, "minecraft:iron_helmet".into(), 1, 0)
        ]
    );
    assert_eq!(
        (back.xp_level, back.xp_progress, back.xp_total),
        (3, 0.5, 30)
    );
    assert_eq!(back.dimension, "minecraft:the_nether");
    assert_eq!(back.world_spawn, Some([8, 64, 8]));
    assert_eq!(back.world_spawn_angle, [45.0, -10.0]);
    assert_eq!(back.spawn, Some([8.0, 64.0, 8.0]));
    assert_eq!(
        back.respawn_elsewhere,
        Some(("minecraft:overworld".into(), [20, 70, 20], 0.0, 0.0, true))
    );
    assert!(back.difficulty_locked);
    assert!(back.weather.raining);
    assert_eq!(back.weather.rain_time, 500);
    assert_eq!(back.game_rule("keep_inventory"), 1);
    assert_eq!(back.game_rule("random_tick_speed"), 3);
    // Unknown player data survives another save.
    let mut extra = back.player_extra.clone().unwrap();
    put(
        &mut extra,
        "ShoulderEntityLeft",
        Tag::compound([("id", string("minecraft:parrot"))]),
    );
    let mut ender = vec![None; 27];
    ender[3] = Some(crate::items::ItemStack::new("minecraft:diamond", 2));
    let mut again = Level {
        player_extra: Some(extra),
        ender_items: ender,
        // The live amount, not the one the loaded compound held (#1307).
        absorption: 2.0,
        ..back
    };
    again.write(&dir).unwrap();
    let third = Level::read(&dir).unwrap();
    assert_eq!(third.absorption, 2.0);
    assert!(matches!(
        third
            .player_extra
            .as_ref()
            .and_then(|p| p.get("ShoulderEntityLeft")),
        Some(Tag::Compound(_))
    ));
    // EnderItems round-trip by slot.
    assert_eq!(
        third.ender_items[3].as_ref().map(|s| (&*s.item, s.count)),
        Some(("minecraft:diamond", 2))
    );
    // A dead player's empty inventory replaces the previously loaded items.
    let mut dead = third;
    dead.health = 0.0;
    dead.inventory.clear();
    dead.inventory_components.clear();
    dead.write(&dir).unwrap();
    let dead = Level::read(&dir).unwrap();
    assert_eq!(dead.health, 0.0);
    assert!(dead.inventory.is_empty());
    assert!(dead.inventory_components.is_empty());
    assert!(dead.player_extra.unwrap().get("equipment").is_none());
    assert!(dir.join("data/minecraft/world_gen_settings.dat").exists());
    let _ = fs::remove_dir_all(&dir);
}

/// An item nested past the writer's depth bound (a mod's component or an
/// `/item ... custom_data`): the player file keeps the rest of the player
/// instead of failing whole (#18384).
#[test]
fn an_item_past_the_save_depth_limit_is_dropped_from_the_player_file() {
    let dir = std::env::temp_dir().join(format!("horizon-deep-item-save-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    // One compound per level, nested here by a loop: the item's depth is
    // this test's own, and only the bound it passes comes from `Limits`.
    let mut deep = Tag::compound([("bottom", Tag::Int(1))]);
    for _ in 0..Limits::saved().max_depth {
        deep = Tag::compound([("nested", deep)]);
    }
    let mut lost = crate::items::ItemStack::new("minecraft:stone", 1);
    lost.data_mut()
        .other
        .push(("minecraft:custom_data".into(), deep));
    let mut ender = vec![None; 27];
    ender[0] = Some(lost);
    ender[1] = Some(crate::items::ItemStack::new("minecraft:diamond", 2));
    let mut level = Level {
        inventory: vec![(0, "minecraft:torch".into(), 4, 0)],
        ender_items: ender,
        ..Level::default()
    };
    level.write_player(&dir).unwrap();

    let file = dir
        .join("players")
        .join("data")
        .join(format!("{}.dat", uuid_text(level.player_uuid.unwrap())));
    let saved = read_dat(&file).unwrap();
    let ender = decode_items(saved.get("EnderItems"), 27);
    assert_eq!(ender[0], None);
    assert_eq!(
        ender[1].as_ref().map(|s| (&*s.item, s.count)),
        Some(("minecraft:diamond", 2))
    );
    let inventory = decode_items(saved.get("Inventory"), 36);
    assert_eq!(
        inventory[0].as_ref().map(|s| (&*s.item, s.count)),
        Some(("minecraft:torch", 4))
    );
    let _ = fs::remove_dir_all(&dir);
}

fn eff(
    id: &str,
    duration: i32,
    amplifier: i32,
    ambient: bool,
    particles: bool,
    icon: bool,
) -> crate::effects::SavedEffect {
    crate::effects::SavedEffect {
        ambient,
        particles,
        icon,
        ..crate::effects::SavedEffect::new(id, duration, amplifier)
    }
}

#[test]
fn a_player_keeps_the_effects_hidden_under_a_stronger_one() {
    // MobEffectInstance.Details.CODEC nests `hidden_effect` (no id of its
    // own): Poison II for 6 s over Poison I for 30 s over Poison I
    // forever, as `/effect give` leaves them.
    let tag = Tag::compound([(
        "active_effects",
        Tag::List(vec![Tag::compound([
            ("id", string("minecraft:poison")),
            ("duration", Tag::Int(120)),
            ("amplifier", Tag::Byte(1)),
            ("show_icon", Tag::Byte(1)),
            (
                "hidden_effect",
                Tag::compound([
                    ("duration", Tag::Int(600)),
                    ("ambient", Tag::Byte(1)),
                    ("show_icon", Tag::Byte(1)),
                    (
                        "hidden_effect",
                        Tag::compound([
                            ("duration", Tag::Int(-1)),
                            ("show_particles", Tag::Byte(0)),
                            ("show_icon", Tag::Byte(0)),
                        ]),
                    ),
                ]),
            ),
        ])]),
    )]);
    let mut level = Level::default();
    level.read_player(&tag);
    let expected = crate::effects::SavedEffect {
        hidden: Some(Box::new(crate::effects::SavedEffect {
            hidden: Some(Box::new(eff(
                "minecraft:poison",
                -1,
                0,
                false,
                false,
                false,
            ))),
            ..eff("minecraft:poison", 600, 0, true, true, true)
        })),
        ..eff("minecraft:poison", 120, 1, false, true, true)
    };
    assert_eq!(level.active_effects, [expected]);
    // Written back, the chain is the same compound.
    assert_eq!(
        level.player_tag().get("active_effects"),
        tag.get("active_effects")
    );
}

#[test]
fn a_saved_effect_without_show_icon_takes_show_particles() {
    // MobEffectInstance.Details.CODEC: `show_icon` defaults to
    // `show_particles`; an explicit one is kept apart from it.
    let effect = |extra: Vec<(&'static str, Tag)>| {
        let mut e = vec![
            ("id", string("minecraft:darkness")),
            ("duration", Tag::Int(260)),
        ];
        e.extend(extra);
        Tag::compound(e)
    };
    let mut level = Level::default();
    level.read_player(&Tag::compound([(
        "active_effects",
        Tag::List(vec![
            effect(vec![("show_particles", Tag::Byte(0))]),
            effect(vec![]),
            effect(vec![("show_icon", Tag::Byte(0))]),
        ]),
    )]));
    let flags: Vec<_> = level
        .active_effects
        .iter()
        .map(|e| (e.particles, e.icon))
        .collect();
    assert_eq!(flags, [(false, false), (true, true), (true, false)]);
}

#[test]
fn effects_that_ended_are_not_saved_again() {
    // A player loaded with an effect that has since run out: the loaded
    // compound still holds it, but the save writes only what is active.
    let mut level = Level {
        active_effects: vec![eff("minecraft:speed", 200, 1, false, true, true)],
        ..Level::default()
    };
    let loaded = level.player_tag();
    assert!(loaded.get("active_effects").is_some());
    level.player_extra = Some(loaded);
    level.active_effects.clear();
    assert!(level.player_tag().get("active_effects").is_none());
}

#[test]
fn effect_amplifier_default_is_omitted_from_saved_players() {
    let effects = vec![
        eff("minecraft:luck", -1, 0, false, false, false),
        eff("minecraft:speed", 200, 1, true, false, true),
        eff("minecraft:regeneration", 40, 128, false, true, false),
        eff("minecraft:strength", -1, 255, false, true, true),
    ];
    for raw in [
        None,
        Some(Tag::compound([
            ("H100Keep", Tag::Int(7)),
            (
                "active_effects",
                Tag::List(vec![Tag::compound([
                    ("id", string("minecraft:luck")),
                    ("amplifier", Tag::Byte(42)),
                ])]),
            ),
        ])),
    ] {
        let level = Level {
            active_effects: effects.clone(),
            player_extra: raw.clone(),
            ..Level::default()
        };
        let saved = level.player_tag();
        let Some(Tag::List(entries)) = saved.get("active_effects") else {
            panic!("modeled live effects must be saved even without raw input")
        };
        assert_eq!(entries.len(), 4);
        assert_eq!(
            entries
                .iter()
                .map(|e| e.get("amplifier").cloned())
                .collect::<Vec<_>>(),
            vec![
                None,
                Some(Tag::Byte(1)),
                Some(Tag::Byte(-128)),
                Some(Tag::Byte(-1))
            ],
        );
        for (entry, e) in entries.iter().zip(&effects) {
            assert_eq!(entry.get("id"), Some(&string(&e.id)));
            assert_eq!(entry.get("duration"), Some(&Tag::Int(e.duration)));
            // MobEffectInstance.Details.MAP_CODEC leaves out ambient false
            // and show_particles true.
            let ambient_tag = e.ambient.then_some(Tag::Byte(1));
            assert_eq!(entry.get("ambient"), ambient_tag.as_ref());
            let particles_tag = (!e.particles).then_some(Tag::Byte(0));
            assert_eq!(entry.get("show_particles"), particles_tag.as_ref());
            assert_eq!(entry.get("show_icon"), Some(&Tag::Byte(i8::from(e.icon))));
        }
        assert_eq!(
            level.active_effects, effects,
            "saving must not mutate live state"
        );
        assert_eq!(
            saved.get("H100Keep"),
            raw.as_ref().and_then(|t| t.get("H100Keep"))
        );
    }
}

#[test]
fn effect_amplifier_255_reloads_as_an_unsigned_byte() {
    // `MobEffectInstance.MAX_AMPLIFIER` is 255. `Details.MAP_CODEC` writes
    // it with `ExtraCodecs.UNSIGNED_BYTE`: `255.byteValue()` is -1 and
    // `128.byteValue()` is -128; `UnsignedBytes.toInt` restores both.
    let effects = vec![
        eff("minecraft:strength", -1, 255, false, true, true),
        eff("minecraft:regeneration", -1, 128, false, true, true),
        eff("minecraft:speed", 40, 1, false, true, true),
    ];
    let level = Level {
        active_effects: effects.clone(),
        ..Level::default()
    };
    let tag = level.player_tag();
    let Tag::List(saved) = tag.get("active_effects").expect("active_effects") else {
        panic!("active_effects list");
    };
    assert_eq!(
        saved
            .iter()
            .map(|e| e.get("amplifier").cloned())
            .collect::<Vec<_>>(),
        vec![
            Some(Tag::Byte(-1)),
            Some(Tag::Byte(-128)),
            Some(Tag::Byte(1)),
        ]
    );
    let mut loaded = Level::default();
    loaded.read_player(&tag);
    assert_eq!(loaded.active_effects, effects);
}

#[test]
fn forced_overworld_spawnpoint_reloads_as_a_configuration() {
    // A personal point coinciding with the world spawn must still keep
    // its forced flag and yaw, rather than turning into a world fallback.
    for pos in [[5, 64, 5], [0, 80, 0]] {
        let point = ("minecraft:overworld".to_owned(), pos, 37.0, 0.0, true);
        let level = Level {
            respawn_elsewhere: Some(point.clone()),
            ..Level::default()
        };
        let mut loaded = Level {
            spawn: Some([0.0, 80.0, 0.0]),
            world_spawn: Some([0, 80, 0]),
            ..Level::default()
        };
        loaded.read_player(&level.player_tag());
        assert_eq!(loaded.respawn_elsewhere, Some(point));
        assert_eq!(loaded.spawn, Some([0.0, 80.0, 0.0]));
        assert_eq!(
            loaded.player_tag().get("respawn"),
            level.player_tag().get("respawn")
        );
    }
}

#[test]
fn creative_flight_status_persists_in_save_and_resets_for_survival() {
    let mut level = Level {
        creative: true,
        flying: true,
        ..Level::default()
    };
    let tag = level.player_tag();
    let flying_byte = tag
        .get("abilities")
        .and_then(|a| number(a.get("flying")))
        .unwrap_or(0);
    assert_eq!(flying_byte, 1);

    let mut loaded = Level::default();
    loaded.read_player(&tag);
    assert!(loaded.flying);

    // Survival mode resets flying ability to 0 in save.
    level.creative = false;
    level.game_mode = 0;
    let tag_survival = level.player_tag();
    let flying_survival = tag_survival
        .get("abilities")
        .and_then(|a| number(a.get("flying")))
        .unwrap_or(0);
    assert_eq!(flying_survival, 0);
}

#[test]
fn unconsumed_root_vehicle_keeps_the_complete_vanilla_tree() {
    let entity = Tag::compound([
        ("id", Tag::String("minecraft:horse".into())),
        ("UUID", uuid_tag([5, 6, 7, 8])),
        (
            "Passengers",
            Tag::List(vec![Tag::compound([
                ("id", Tag::String("minecraft:pig".into())),
                ("UUID", uuid_tag([1, 2, 3, 4])),
            ])]),
        ),
    ]);
    let vehicle = Tag::compound([("Attach", uuid_tag([1, 2, 3, 4])), ("Entity", entity)]);
    let raw = Tag::compound([("RootVehicle", vehicle.clone())]);
    let mut level = Level::default();
    level.read_player(&raw);
    level.player_extra = Some(raw);
    assert_eq!(level.root_vehicle, Some([1, 2, 3, 4]));
    assert_eq!(level.player_tag().get("RootVehicle"), Some(&vehicle));
    // Even saving before the vehicle is restored must not destroy it.
    level.root_vehicle = None;
    assert_eq!(level.player_tag().get("RootVehicle"), Some(&vehicle));
}

/// `RecipesUsed` is what a furnace owes in experience, so a save that drops it pays the
/// player nothing after a reload.
#[test]
fn a_furnaces_recipes_used_survive_a_save() {
    use crate::containers::BlockEntity;
    let Some(BlockEntity::Furnace(mut f)) = BlockEntity::for_block("minecraft:furnace") else {
        panic!("a furnace");
    };
    f.recipes_used
        .insert("minecraft:cooked_porkchop".to_owned(), 3);
    let tag =
        block_entity_encode::encode((1, 64, 2), &BlockEntity::Furnace(f), "minecraft:furnace");
    let Some((_, BlockEntity::Furnace(back))) = decode_block_entity(&tag) else {
        panic!("a furnace back");
    };
    assert_eq!(back.recipes_used.get("minecraft:cooked_porkchop"), Some(&3));
}
/// The world's save (Level.write) writes the rules as they are now, and a
/// world opened again reads them back (#74).
#[test]
fn level_save_keeps_changed_rules() {
    use crate::gamerules::{GameRules, Value};
    let dir = std::env::temp_dir().join(format!("horizon-gamerules-level-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut level = super::Level::default();
    level.write(&dir).unwrap();
    // Rules changed while playing, then saved and loaded again.
    let mut rules = GameRules::load(&dir);
    assert_eq!(rules, GameRules::default());
    assert!(rules.set("advance_time", Value::Bool(false)));
    assert!(rules.set("advance_weather", Value::Bool(false)));
    assert!(rules.set("random_tick_speed", Value::Int(0)));
    level.game_rules = rules.by_name();
    level.write(&dir).unwrap();
    assert_eq!(GameRules::load(&dir), rules);
    let read = super::Level::read(&dir).unwrap();
    assert_eq!(read.game_rules, rules.by_name());
    assert_eq!(read.game_rule("advance_time"), 0);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn level_save_rule_files_round_trip_without_byte_changes() {
    for absent_mod in [false, true] {
        let dir = std::env::temp_dir().join(format!(
            "horizon-rule-file-roundtrip-{}-{absent_mod}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let mut level = super::Level::default();
        level.write(&dir).unwrap();
        if absent_mod {
            let mut data = super::read_saved_data(&dir, "game_rules").unwrap();
            super::put(&mut data, "absent:rule", super::Tag::Long(42));
            super::write_saved_data(&dir, "game_rules", data).unwrap();
        }
        let mut before_level = super::read_dat(&dir.join("level.dat")).unwrap();
        let path = super::saved_data_path(&dir, "game_rules");
        let before_rules = std::fs::read(&path).unwrap();
        super::Level::read(&dir).unwrap().write(&dir).unwrap();
        let mut after_level = super::read_dat(&dir.join("level.dat")).unwrap();
        for level in [&mut before_level, &mut after_level] {
            if let super::Tag::Compound(root) = level
                && let Some(super::Tag::Compound(data)) = root.get_mut(&"Data".into())
            {
                data.remove(&"LastPlayed".into());
            }
        }
        assert_eq!(after_level, before_level);
        assert_eq!(std::fs::read(path).unwrap(), before_rules);
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[test]
fn level_read_retains_the_session_registry() {
    let dir = std::env::temp_dir().join(format!("horizon-level-registry-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let first = std::sync::Arc::new(crate::gamerules::RuleRegistry::vanilla());
    let second = std::sync::Arc::new(crate::gamerules::RuleRegistry::vanilla());
    let mut level = super::Level {
        rule_registry: first.clone(),
        ..Default::default()
    };
    level.write(&dir).unwrap();
    let loaded = super::Level::read_with_registry(&dir, first.clone()).unwrap();
    assert!(std::sync::Arc::ptr_eq(&loaded.rule_registry, &first));
    let next = super::Level::read_with_registry(&dir, second.clone()).unwrap();
    assert!(!std::sync::Arc::ptr_eq(
        &loaded.rule_registry,
        &next.rule_registry
    ));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn level_saved_rules_have_vanilla_keys_types_and_defaults() {
    let dir = std::env::temp_dir().join(format!("horizon-rule-defaults-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut level = super::Level::default();
    level.write(&dir).unwrap();
    let super::Tag::Compound(rules) = super::read_saved_data(&dir, "game_rules").unwrap() else {
        panic!("rule compound")
    };
    assert_eq!(rules.len(), 58);
    assert!(
        rules
            .keys()
            .all(|key| key.to_string().unwrap().starts_with("minecraft:"))
    );
    assert_eq!(
        rules.get(&"minecraft:random_tick_speed".into()),
        Some(&super::Tag::Int(3))
    );
    assert_eq!(
        rules.get(&"minecraft:keep_inventory".into()),
        Some(&super::Tag::Byte(0))
    );
    assert_eq!(
        rules.get(&"minecraft:max_snow_accumulation_height".into()),
        Some(&super::Tag::Int(1))
    );
    assert!(!rules.contains_key(&"minecraft:max_minecart_speed".into()));
    level.enabled_features = [crate::gamerules::MINECART_IMPROVEMENTS.to_owned()].into();
    level.game_rules =
        crate::gamerules::GameRules::with_features(&level.enabled_features).by_name();
    level.write(&dir).unwrap();
    assert_eq!(
        super::read_saved_data(&dir, "game_rules")
            .unwrap()
            .get("minecraft:max_minecart_speed"),
        Some(&super::Tag::Int(8))
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn legacy_bare_saved_rules_load_under_canonical_keys() {
    let dir = std::env::temp_dir().join(format!("horizon-rule-legacy-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    super::Level::default().write(&dir).unwrap();
    super::write_saved_data(
        &dir,
        "game_rules",
        super::Tag::compound([
            ("keep_inventory", super::Tag::Byte(1)),
            ("random_tick_speed", super::Tag::Int(7)),
        ]),
    )
    .unwrap();
    let level = super::Level::read(&dir).unwrap();
    assert_eq!(level.game_rules.get("minecraft:keep_inventory"), Some(&1));
    assert_eq!(
        level.game_rules.get("minecraft:random_tick_speed"),
        Some(&7)
    );
    assert!(!level.game_rules.contains_key("keep_inventory"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn level_save_preserves_absent_mod_rules() {
    let dir =
        std::env::temp_dir().join(format!("horizon-gamerules-unknown-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut level = super::Level::default();
    level.write(&dir).unwrap();
    super::write_saved_data(
        &dir,
        "game_rules",
        super::Tag::compound([("absent:rule", super::Tag::Long(42))]),
    )
    .unwrap();
    level.game_rules.insert("minecraft:advance_time".into(), 0);
    level.write(&dir).unwrap();
    assert_eq!(
        super::read_saved_data(&dir, "game_rules")
            .unwrap()
            .get("absent:rule"),
        Some(&super::Tag::Long(42))
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// A save written before the key was namespaced holds an id twice, once bare: vanilla
/// refuses such a furnace, so a load merges them under the full id.
#[test]
fn a_furnaces_recipes_used_merges_bare_and_full_ids() {
    use crate::containers::BlockEntity;
    let Some(BlockEntity::Furnace(mut f)) = BlockEntity::for_block("minecraft:furnace") else {
        panic!("a furnace");
    };
    f.recipes_used
        .insert("iron_ingot_from_smelting_iron_ore".to_owned(), 2);
    f.recipes_used
        .insert("minecraft:iron_ingot_from_smelting_iron_ore".to_owned(), 3);
    let tag =
        block_entity_encode::encode((1, 64, 2), &BlockEntity::Furnace(f), "minecraft:furnace");
    let Some((_, BlockEntity::Furnace(back))) = decode_block_entity(&tag) else {
        panic!("a furnace back");
    };
    assert_eq!(back.recipes_used.len(), 1);
    assert_eq!(
        back.recipes_used
            .get("minecraft:iron_ingot_from_smelting_iron_ore"),
        Some(&5)
    );
}

/// `WorldDataConfiguration.MAP_CODEC`'s `lenientOptionalFieldOf
/// ("enabled_features", DEFAULT_FLAGS)`: a picked experiment's list
/// round-trips, a vanilla-only selection leaves the tag out.
#[test]
fn enabled_features_round_trip_and_omit_the_vanilla_default() {
    let dir = gen_world("enabled-features");
    let mut level = Level {
        name: "Features".into(),
        enabled_features: [
            "minecraft:vanilla".into(),
            "minecraft:minecart_improvements".into(),
        ]
        .into(),
        ..Level::default()
    };
    level.write(&dir).unwrap();
    let back = Level::read(&dir).unwrap();
    assert_eq!(back.enabled_features, level.enabled_features);
    // The tag is in the file.
    let raw = crate::save::read_level_dat(&dir).unwrap();
    if let Tag::Compound(data) = &raw {
        let data = data.get(&JavaString::from("Data")).unwrap();
        assert!(matches!(data.get("enabled_features"), Some(Tag::List(_))));
    }
    // A vanilla-only selection writes no tag, and reads back as the
    // same vanilla-only list. A fresh folder: write_internal keeps an
    // older file's fields.
    let dir2 = gen_world("enabled-features-vanilla");
    level.enabled_features = ["minecraft:vanilla".into()].into();
    level.write(&dir2).unwrap();
    let raw = crate::save::read_level_dat(&dir2).unwrap();
    if let Tag::Compound(data) = &raw {
        let data = data.get(&JavaString::from("Data")).unwrap();
        assert!(data.get("enabled_features").is_none());
    }
    let back = Level::read(&dir2).unwrap();
    // The read side keeps its own convention: empty is the vanilla
    // default the file omitted.
    assert!(back.enabled_features.is_empty());
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(dir2);
}

fn gen_world(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("horizon-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// `WorldOpenFlows.openWorldCheckWorldStemCompatibility`: every world a preset makes is
/// stable, and a world with a changed dimension, a custom one or a customized generator
/// is asked about.
#[test]
fn world_stem_compatibility_flags_unstable_and_customized_worlds() {
    for preset in [
        WorldPreset::Normal,
        WorldPreset::Flat,
        WorldPreset::LargeBiomes,
        WorldPreset::Amplified,
        WorldPreset::SingleBiomeSurface,
        WorldPreset::Debug,
    ] {
        let dir = gen_world("stem-stable");
        write_world_gen_settings(
            &dir,
            7,
            WorldGen {
                preset,
                structures: true,
                bonus_chest: false,
            },
        )
        .unwrap();
        assert!(!world_gen_unstable(&dir), "{preset:?}");
        assert!(!is_old_customized_world(&dir));
        let _ = fs::remove_dir_all(dir);
    }
    // Without world_gen_settings there is nothing to ask about.
    let dir = gen_world("stem-none");
    assert!(!world_gen_unstable(&dir) && !is_old_customized_world(&dir));
    let _ = fs::remove_dir_all(dir);

    let dir = gen_world("stem-edit");
    write_world_gen_settings(&dir, 7, WorldGen::default()).unwrap();
    let settings = world_gen_settings(&dir).unwrap();
    let edit = |change: &dyn Fn(&mut Tag)| {
        let mut tag = settings.clone();
        change(&mut tag);
        write_saved_data(&dir, "world_gen_settings", tag).unwrap();
    };
    fn dimension<'a>(tag: &'a mut Tag, key: &str) -> &'a mut Tag {
        tag.get_mut("dimensions").unwrap().get_mut(key).unwrap()
    }
    // A custom biome source in the Nether is not the preset.
    edit(&|tag| {
        let nether = dimension(tag, "minecraft:the_nether");
        let generator = nether.get_mut("generator").unwrap();
        *generator.get_mut("biome_source").unwrap() = Tag::compound([
            ("type", string("minecraft:fixed")),
            ("biome", string("minecraft:plains")),
        ]);
    });
    assert!(world_gen_unstable(&dir));
    // The End on another dimension type.
    edit(&|tag| {
        let end = dimension(tag, "minecraft:the_end");
        *end.get_mut("type").unwrap() = string("minecraft:overworld");
    });
    assert!(world_gen_unstable(&dir));
    // Another noise settings in the Overworld is fine (amplified), in the Nether it is not.
    edit(&|tag| {
        let nether = dimension(tag, "minecraft:the_nether");
        *nether
            .get_mut("generator")
            .unwrap()
            .get_mut("settings")
            .unwrap() = string("minecraft:overworld");
    });
    assert!(world_gen_unstable(&dir));
    // A fourth dimension, and a missing builtin one.
    edit(&|tag| {
        if let Some(Tag::Compound(dimensions)) = tag.get_mut("dimensions") {
            let end = dimensions.get(&"minecraft:the_end".into()).unwrap().clone();
            dimensions.insert("mypack:moon".into(), end);
        }
    });
    assert!(world_gen_unstable(&dir));
    edit(&|tag| {
        if let Some(Tag::Compound(dimensions)) = tag.get_mut("dimensions") {
            dimensions.remove(&"minecraft:the_end".into());
        }
    });
    assert!(world_gen_unstable(&dir));
    // legacy_custom_options marks an old customized world.
    edit(&|tag| {
        if let Tag::Compound(map) = tag {
            map.insert("legacy_custom_options".into(), string("{\"seaLevel\":63}"));
        }
    });
    assert!(is_old_customized_world(&dir));
    let _ = fs::remove_dir_all(dir);
}

/// `WorldOpenFlows.loadWorldStem`: the enabled data packs' level stems are the base of
/// `WorldDimensions.bake`, replacing the saved one of their id.
#[test]
fn world_stem_compatibility_reads_data_pack_dimensions() {
    let dir = gen_world("stem-datapack");
    write_world_gen_settings(&dir, 7, WorldGen::default()).unwrap();
    assert!(!world_gen_unstable(&dir));
    let write = |pack: &str, file: &str, text: &str| {
        let path = dir.join("datapacks").join(pack).join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    };
    write("p", "pack.mcmeta", r#"{"pack":{"description":"fixture"}}"#);
    // A pack's dimension of another id is not vanilla-like.
    write(
        "p",
        "data/mypack/dimension/moon.json",
        r#"{"type":"minecraft:the_end","generator":{"type":"minecraft:noise","settings":"minecraft:end","biome_source":{"type":"minecraft:the_end"}}}"#,
    );
    assert!(world_gen_unstable(&dir));
    // Disabled, it counts for nothing.
    fs::write(dir.join("datapacks.json"), r#"{"disabled":["file/p"]}"#).unwrap();
    assert!(!world_gen_unstable(&dir));
    fs::remove_file(dir.join("datapacks.json")).unwrap();
    fs::remove_file(dir.join("datapacks/p/data/mypack/dimension/moon.json")).unwrap();
    assert!(!world_gen_unstable(&dir));
    // A pack's Nether with another biome source replaces the saved vanilla one.
    write(
        "p",
        "data/minecraft/dimension/the_nether.json",
        r#"{"type":"minecraft:the_nether","generator":{"type":"minecraft:noise","settings":"minecraft:nether","biome_source":{"type":"minecraft:fixed","biome":"minecraft:plains"}}}"#,
    );
    assert!(world_gen_unstable(&dir));
    // And a vanilla-like one stays stable.
    write(
        "p",
        "data/minecraft/dimension/the_nether.json",
        r#"{"type":"minecraft:the_nether","generator":{"type":"minecraft:noise","settings":"minecraft:nether","biome_source":{"type":"minecraft:multi_noise","preset":"minecraft:nether"}}}"#,
    );
    assert!(!world_gen_unstable(&dir));
    let _ = fs::remove_dir_all(dir);
}

/// A world that still needs its file fixes keeps its generator settings in level.dat; they
/// are read as the fixes would leave them, so the question is asked on the first open.
#[test]
fn world_gen_settings_of_a_world_needing_file_fixes_are_read_in_memory() {
    let dir = gen_world("stem-prefix");
    write_world_gen_settings(&dir, 7, WorldGen::default()).unwrap();
    let settings = world_gen_settings(&dir).unwrap();
    let customized = {
        let mut tag = settings.clone();
        if let Tag::Compound(map) = &mut tag {
            map.insert("legacy_custom_options".into(), string("{\"seaLevel\":63}"));
        }
        tag
    };
    // 26.3 layout moved back to the old one: settings in level.dat, no saved data file.
    fs::remove_file(saved_data_path(&dir, "world_gen_settings")).unwrap();
    write_dat(
        &dir.join("level.dat"),
        Tag::compound([(
            "Data",
            Tag::compound([
                ("DataVersion", Tag::Int(4700)),
                ("WorldGenSettings", customized),
            ]),
        )]),
    )
    .unwrap();
    assert!(needs_file_fixes(&dir));
    let read = world_gen_settings(&dir).expect("settings");
    assert!(read.get("dimensions").is_some());
    assert!(!world_gen_unstable(&dir));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn low_disk_space_is_under_64_mib_usable() {
    assert_eq!(LOW_DISK_SPACE, 64 * 1024 * 1024);
    let dir = std::env::temp_dir();
    assert!(estimate_disk_space(&dir) > 0);
    assert_eq!(
        low_disk_space(&dir),
        estimate_disk_space(&dir) < LOW_DISK_SPACE
    );
    // A path that does not exist cannot be asked: vanilla reads that as plenty.
    #[cfg(unix)]
    assert_eq!(
        estimate_disk_space(&dir.join("horizon-no-such-folder/x")),
        u64::MAX
    );
}

#[test]
fn mod_attachments_save_in_horizons_namespace_only_when_some() {
    use horizon_mod_runtime::attachments::Attachments;
    let root = std::env::temp_dir().join(format!("horizon-attach-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let save = SaveDir::new(root.clone(), LevelShape::OVERWORLD);
    let file = root.join("data/horizon/attachments.dat");
    save.save_attachments(&Attachments::default()).unwrap();
    assert!(
        !root.join("data").exists(),
        "a level without mod data writes nothing"
    );
    let data = Compound::from_iter([("example:count".into(), Tag::Int(3))]);
    save.save_attachments(&Attachments::from_nbt(data.clone()))
        .unwrap();
    assert!(file.is_file());
    assert_eq!(save.load_attachments().to_nbt(), data);
    fs::write(&file, b"not nbt").unwrap();
    assert!(save.load_attachments().to_nbt().is_empty());
    assert_eq!(
        fs::read(root.join("data/horizon/attachments.dat.corrupt")).unwrap(),
        b"not nbt"
    );
    assert!(!file.exists());
    fs::remove_dir_all(&root).unwrap();
}

/// A level id never names a folder outside the world's `dimensions` folder, and no two ids
/// share a folder.
#[test]
fn a_level_folder_stays_inside_the_world() {
    let world = Path::new("/w");
    assert_eq!(
        dimension_dir(world, "minecraft:the_nether"),
        Path::new("/w/dimensions/minecraft/the_nether")
    );
    assert_eq!(
        dimension_dir(world, "test:a/b"),
        Path::new("/w/dimensions/test/a/b")
    );
    let mut seen = std::collections::HashSet::new();
    for id in [
        "test:../minecraft/overworld",
        "test:a/../../b",
        "test:/abs",
        "..:x",
        "test:a//b",
        "test:Pocket",
    ] {
        let dir = dimension_dir(world, id);
        assert!(
            dir.starts_with("/w/dimensions/%invalid")
                && dir
                    .components()
                    .all(|c| !matches!(c, std::path::Component::ParentDir)),
            "{id}: {}",
            dir.display()
        );
        assert!(seen.insert(dir), "{id} shares a folder");
    }
}

#[test]
fn forced_chunks_survive_dimension_restarts_and_removals() {
    let root = std::env::temp_dir().join(format!("horizon-forced-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let dimensions = [
        "minecraft:overworld",
        "minecraft:the_nether",
        "minecraft:the_end",
    ];
    for (i, dim) in dimensions.iter().enumerate() {
        let save = SaveDir::new(dimension_dir(&root, dim), LevelShape::OVERWORLD);
        assert!(save.load_forced_chunks().is_empty());
        let forced = HashSet::from([(i as i32, -17), (-23, i as i32), (-1, -1)]);
        save.save_chunk_tickets(&forced, &[]).unwrap();
    }
    for (i, dim) in dimensions.iter().enumerate() {
        let path = dimension_dir(&root, dim);
        let save = SaveDir::new(path.clone(), LevelShape::OVERWORLD);
        let mut forced = save.load_forced_chunks();
        assert_eq!(
            forced,
            HashSet::from([(i as i32, -17), (-23, i as i32), (-1, -1)])
        );
        let mut data = read_saved_data(&path, "chunk_tickets").unwrap();
        let other = Tag::compound([
            ("type", Tag::String("example:loader".into())),
            ("level", Tag::Int(30)),
            ("chunk_pos", Tag::IntArray(vec![7, -8])),
            ("ticks_left", Tag::Long(42)),
        ]);
        if let Some(Tag::List(tickets)) = data.get_mut("tickets") {
            assert!(
                tickets
                    .iter()
                    .all(|t| t.get("level") == Some(&Tag::Int(31)))
            );
            tickets.push(other.clone());
        }
        write_saved_data(&path, "chunk_tickets", data).unwrap();
        forced.remove(&(-1, -1));
        save.save_chunk_tickets(&forced, &[]).unwrap();
        drop(save);
        let reopened = SaveDir::new(path.clone(), LevelShape::OVERWORLD);
        assert_eq!(reopened.load_forced_chunks(), forced);
        reopened.save_chunk_tickets(&HashSet::new(), &[]).unwrap();
        assert!(
            SaveDir::new(path.clone(), LevelShape::OVERWORLD)
                .load_forced_chunks()
                .is_empty()
        );
        assert_eq!(
            read_saved_data(&path, "chunk_tickets")
                .unwrap()
                .get("tickets"),
            Some(&Tag::List(vec![other]))
        );
    }
    fs::remove_dir_all(root).unwrap();
}

/// The Debug preset (`worldgen/world_preset/debug_all_block_states.json`): its id names it,
/// the saved overworld generator is `minecraft:debug` and reads back as it, and the create
/// screen's button never cycles to it.
#[test]
fn the_debug_preset_is_saved_and_read_back() {
    assert_eq!(
        WorldPreset::of_id("minecraft:debug_all_block_states"),
        Some(WorldPreset::Debug)
    );
    assert!(!WorldPreset::ALL.contains(&WorldPreset::Debug));
    let dir = gen_world("debug-preset");
    write_world_gen_settings(
        &dir,
        7,
        WorldGen {
            preset: WorldPreset::Debug,
            structures: true,
            bonus_chest: false,
        },
    )
    .unwrap();
    let settings = world_gen_settings(&dir).unwrap();
    let generator = settings
        .get("dimensions")
        .and_then(|d| d.get("minecraft:overworld"))
        .and_then(|o| o.get("generator"))
        .unwrap();
    assert_eq!(
        text(generator.get("type")).as_deref(),
        Some("minecraft:debug")
    );
    assert_eq!(WorldGen::read(&dir).unwrap().preset, WorldPreset::Debug);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn world_preset_next_and_prev_cycle_bidirectionally() {
    let first = WorldPreset::Normal;
    assert_eq!(first.next(), WorldPreset::Flat);
    assert_eq!(first.prev(), WorldPreset::SingleBiomeSurface);
    assert_eq!(WorldPreset::SingleBiomeSurface.next(), WorldPreset::Normal);
    for preset in WorldPreset::ALL {
        assert_eq!(preset.next().prev(), preset);
        assert_eq!(preset.prev().next(), preset);
    }
}
/// A region record of the dimension folder `save` at chunk (0, 0), written
/// as an older version left it.
fn write_old_record(save: &SaveDir, folder: &str, tag: Tag) {
    let record = Record::Tag(Arc::new(NamedTag {
        name: "".into(),
        tag,
    }));
    let cache = match Folder::of(folder) {
        Folder::Region => &save.regions,
        Folder::Entities => &save.entity_regions,
        Folder::Poi => &save.poi_regions,
    };
    super::write_region_records(&save.root, cache, folder, &[((0, 0), &record)]).unwrap();
}

#[test]
fn older_records_of_a_current_world_upgrade_as_they_are_read() {
    let world = std::env::temp_dir().join(format!("horizon-lazy-dv-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&world);
    let save = SaveDir::new(
        super::dimension_dir(&world, "minecraft:overworld"),
        LevelShape::OVERWORLD,
    );
    // A 1.20.1 chunk (DataVersion 3465): 1.20.3 renamed its grass to short_grass.
    let section = Tag::compound([
        ("Y", Tag::Byte(-4)),
        (
            "block_states",
            Tag::compound([(
                "palette",
                Tag::List(vec![Tag::compound([("Name", string("minecraft:grass"))])]),
            )]),
        ),
        (
            "biomes",
            Tag::compound([("palette", Tag::List(vec![string("minecraft:plains")]))]),
        ),
    ]);
    write_old_record(
        &save,
        "region",
        Tag::compound([
            ("DataVersion", Tag::Int(3465)),
            ("xPos", Tag::Int(0)),
            ("zPos", Tag::Int(0)),
            ("yPos", Tag::Int(-4)),
            ("Status", string("minecraft:full")),
            ("sections", Tag::List(vec![section])),
        ]),
    );
    let chunk = save.raw_chunk(0, 0).expect("the chunk reads");
    assert_eq!(
        number(chunk.tag.get("DataVersion")),
        Some(i64::from(DATA_VERSION))
    );
    let Some(Tag::List(sections)) = chunk.tag.get("sections") else {
        panic!("no sections: {:?}", chunk.tag);
    };
    let Some(Tag::List(palette)) = sections[0]
        .get("block_states")
        .and_then(|b| b.get("palette"))
    else {
        panic!("no palette");
    };
    assert_eq!(
        text(palette[0].get("id")).as_deref(),
        Some("minecraft:short_grass")
    );
    // Decoded, it keeps its blocks: read raw, the 26.x decoder finds no `id` in a
    // 1.20 palette (`Name`) and the whole column is air.
    if let Some(root) = std::env::var_os("HORIZON_TEST_RESOURCES") {
        let root = PathBuf::from(root);
        let assets = crate::assets::Assets::load(&root).expect("assets");
        let biomes = Biomes::load(&horizon_assets::resource_source::ResourceRoot::directory(
            root.to_path_buf(),
        ))
        .expect("biomes");
        let grass = assets
            .registry
            .default_state("minecraft:short_grass")
            .unwrap();
        let loaded = save
            .load(0, 0, assets.registry, &biomes)
            .expect("the chunk loads");
        assert_eq!(loaded.get(3, -60, 7), grass.0);
    }
    // A 1.20.1 item entity: 1.20.5 made its stack `count` (an int) from `Count`.
    write_old_record(
        &save,
        "entities",
        Tag::compound([
            ("DataVersion", Tag::Int(3465)),
            ("Position", Tag::IntArray(vec![0, 0])),
            (
                "Entities",
                Tag::List(vec![Tag::compound([
                    ("id", string("minecraft:item")),
                    (
                        "Item",
                        Tag::compound([("id", string("minecraft:stone")), ("Count", Tag::Byte(3))]),
                    ),
                ])]),
            ),
        ]),
    );
    let entities = save.load_entities(0, 0);
    let item = entities[0].get("Item").expect("the item stack");
    assert_eq!(item.get("count"), Some(&Tag::Int(3)));
    assert!(item.get("Count").is_none());
    let _ = std::fs::remove_dir_all(&world);
}

#[test]
fn old_chunks_are_found_in_the_saved_records_and_old_poi_upgrade() {
    let world = std::env::temp_dir().join(format!("horizon-lazy-old-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&world);
    let nether = SaveDir::new(
        super::dimension_dir(&world, "minecraft:the_nether"),
        LevelShape::NETHER,
    );
    // A 1.20.1 Nether chunk: upgraded it has no `blending_data` (only Overworld
    // chunks blend) and the current DataVersion, so only the saved record says
    // it is older than blending (4997), as `IOWorker.isOldChunk` reads it.
    write_old_record(
        &nether,
        "region",
        Tag::compound([
            ("DataVersion", Tag::Int(3465)),
            ("xPos", Tag::Int(0)),
            ("zPos", Tag::Int(0)),
            ("yPos", Tag::Int(0)),
            ("Status", string("minecraft:full")),
            ("sections", Tag::List(Vec::new())),
        ]),
    );
    let upgraded = nether.raw_chunk(0, 0).expect("the chunk reads");
    assert!(upgraded.tag.get("blending_data").is_none());
    assert_ne!(nether.old_chunk_bits(0, 0)[0] & 1, 0);
    // One chunk's record tells the same, without the region's other chunks; a chunk this
    // version saved, and one not saved, are not old.
    let current = Record::Tag(Arc::new(NamedTag {
        name: "".into(),
        tag: Tag::compound([
            ("DataVersion", Tag::Int(DATA_VERSION)),
            ("xPos", Tag::Int(1)),
            ("zPos", Tag::Int(0)),
            ("yPos", Tag::Int(0)),
            ("Status", string("minecraft:full")),
            ("sections", Tag::List(Vec::new())),
        ]),
    }));
    super::write_region_records(
        &nether.root,
        &nether.regions,
        "region",
        &[((1, 0), &current)],
    )
    .unwrap();
    assert!(nether.is_old_chunk(0, 0));
    assert!(!nether.is_old_chunk(1, 0));
    assert!(!nether.is_old_chunk(2, 0));
    assert_eq!(nether.old_chunk_bits(0, 0)[0] & 0b110, 0);
    // A POI record of before 1.14.4 (no DataVersion, read as 1945): its
    // sections at the root, which 1946 moved under `Sections`.
    let overworld = SaveDir::new(
        super::dimension_dir(&world, "minecraft:overworld"),
        LevelShape::OVERWORLD,
    );
    write_old_record(
        &overworld,
        "poi",
        Tag::compound([(
            "0",
            Tag::compound([("Valid", Tag::Byte(1)), ("Records", Tag::List(Vec::new()))]),
        )]),
    );
    let poi = overworld.read_record("poi", 0, 0).expect("the POI reads");
    assert!(poi.tag.get("Sections").and_then(|s| s.get("0")).is_some());
    assert_eq!(
        number(poi.tag.get("DataVersion")),
        Some(i64::from(DATA_VERSION))
    );
    let _ = std::fs::remove_dir_all(&world);
}

#[test]
fn a_chunk_from_before_1_18_is_brought_to_the_current_layout() {
    let world = std::env::temp_dir().join(format!("horizon-lazy-117-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&world);
    let save = SaveDir::new(
        super::dimension_dir(&world, "minecraft:overworld"),
        LevelShape::OVERWORLD,
    );
    // A 1.17.1 chunk (DataVersion 2730), its fields under `Level`: section 0 all stone.
    let section = Tag::compound([
        ("Y", Tag::Byte(0)),
        (
            "Palette",
            Tag::List(vec![
                Tag::compound([("Name", string("minecraft:air"))]),
                Tag::compound([("Name", string("minecraft:stone"))]),
            ]),
        ),
        (
            "BlockStates",
            Tag::LongArray(vec![0x1111_1111_1111_1111; 256]),
        ),
    ]);
    write_old_record(
        &save,
        "region",
        Tag::compound([
            ("DataVersion", Tag::Int(2730)),
            (
                "Level",
                Tag::compound([
                    ("xPos", Tag::Int(0)),
                    ("zPos", Tag::Int(0)),
                    ("Status", string("full")),
                    ("Biomes", Tag::IntArray(vec![1; 1024])),
                    ("Sections", Tag::List(vec![section])),
                ]),
            ),
        ]),
    );
    let chunk = save.raw_chunk(0, 0).expect("the chunk reads");
    assert!(
        chunk.tag.get("Level").is_none(),
        "still wrapped: {:?}",
        chunk.tag
    );
    assert_eq!(number(chunk.tag.get("xPos")), Some(0));
    // 1.18's height fix: an Overworld chunk older than the new heights blends with them.
    assert!(chunk.tag.get("blending_data").is_some());
    let _ = std::fs::remove_dir_all(&world);
}

/// A chunk last saved by 1.16.5 (DataVersion 2586) keeps its entities in its own
/// `Level.Entities`: the upgrade moves them to `entities` and the load adds them
/// (`SerializableChunkData.postLoadChunk`, `ServerLevel.addLegacyChunkEntities`), so its
/// minecart is in the world (#14424).
#[test]
#[ignore = "daily"]
fn a_chunk_from_before_1_17_loads_the_entities_it_keeps() {
    let root = PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    let assets = crate::assets::Assets::load(&root).expect("assets");
    let biomes = Biomes::load(&horizon_assets::resource_source::ResourceRoot::directory(
        root.clone(),
    ))
    .expect("biomes");
    let world = std::env::temp_dir().join(format!("horizon-lazy-116-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&world);
    let save = SaveDir::new(
        super::dimension_dir(&world, "minecraft:overworld"),
        LevelShape::OVERWORLD,
    );
    let section = Tag::compound([
        ("Y", Tag::Byte(0)),
        (
            "Palette",
            Tag::List(vec![
                Tag::compound([("Name", string("minecraft:air"))]),
                Tag::compound([("Name", string("minecraft:stone"))]),
            ]),
        ),
        (
            "BlockStates",
            Tag::LongArray(vec![0x1111_1111_1111_1111; 256]),
        ),
    ]);
    let minecart = Tag::compound([
        ("id", string("minecraft:minecart")),
        (
            "Pos",
            Tag::List([3.5f64, 16.0, 7.5].map(|v| Tag::Double(v.to_bits())).into()),
        ),
        ("UUID", Tag::IntArray(vec![1, 2, 3, 4])),
    ]);
    write_old_record(
        &save,
        "region",
        Tag::compound([
            ("DataVersion", Tag::Int(2586)),
            (
                "Level",
                Tag::compound([
                    ("xPos", Tag::Int(0)),
                    ("zPos", Tag::Int(0)),
                    ("Status", string("full")),
                    ("Biomes", Tag::IntArray(vec![1; 1024])),
                    ("Sections", Tag::List(vec![section])),
                    ("Entities", Tag::List(vec![minecart])),
                ]),
            ),
        ]),
    );
    let id = |e: &Tag| text(e.get("id"));
    let chunk = save
        .load(0, 0, assets.registry, &biomes)
        .expect("the chunk loads");
    let ids: Vec<_> = chunk.entities.iter().map(id).collect();
    assert_eq!(ids, [Some("minecraft:minecart".to_owned())]);
    // There is no `entities/` record for it: these are all it has.
    assert!(save.load_entities(0, 0).is_empty());
    let _ = std::fs::remove_dir_all(&world);
}

/// A chunk as Bobby leaves it on disk, its palettes in both shapes: a
/// list of names, and compounds where a name without properties is
/// wrapped as `{"": name}` (a mixed list), read back summarized by far
/// terrain and kept in its own folder.
#[test]
fn far_terrain_reads_and_keeps_bobby_chunks() {
    use crate::server_terrain::{Bobby, DecodeChunk, ServerTerrain};
    let Some(root) = std::env::var_os("HORIZON_TEST_RESOURCES") else {
        return;
    };
    let root = PathBuf::from(root);
    let assets = crate::assets::Assets::load(&root).expect("assets");
    let biomes = Arc::new(
        Biomes::load(&horizon_assets::resource_source::ResourceRoot::directory(
            root.to_path_buf(),
        ))
        .expect("biomes"),
    );
    let facts = crate::blocks::BlockFacts::load(&root, assets.registry).expect("facts");
    let dir = std::env::temp_dir().join(format!("horizon-bobby-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let (own, bobby) = (dir.join("own"), dir.join("bobby"));
    fs::create_dir_all(&bobby).unwrap();
    let name = |s: &str| Tag::String(s.into());
    let biome = Tag::compound([("palette", Tag::List(vec![name("minecraft:plains")]))]);
    let section = |y: i8, states: Tag| {
        Tag::compound([
            ("Y", Tag::Byte(y)),
            ("block_states", states),
            ("biomes", biome.clone()),
            ("SkyLight", Tag::ByteArray(vec![0xff; 2048])),
        ])
    };
    // y 48..64 stone; y 64 an upright oak log, air above.
    let stone = Tag::compound([("palette", Tag::List(vec![name("minecraft:stone")]))]);
    let log = Tag::compound([
        ("id", name("minecraft:oak_log")),
        ("properties", Tag::compound([("axis", name("y"))])),
    ]);
    let mut data = vec![0i64; 256];
    data[..16].fill(0x1111_1111_1111_1111);
    let logs = Tag::compound([
        ("palette", Tag::List(vec![name("minecraft:air"), log])),
        ("data", Tag::LongArray(data)),
    ]);
    let chunk = |version: i32| NamedTag {
        name: "".into(),
        tag: Tag::compound([
            ("isLightOn", Tag::Byte(1)),
            ("Status", name("full")),
            ("zPos", Tag::Int(5)),
            ("block_entities", Tag::List(Vec::new())),
            ("yPos", Tag::Int(-4)),
            ("DataVersion", Tag::Int(version)),
            ("xPos", Tag::Int(-2)),
            (
                "sections",
                Tag::List(vec![section(3, stone.clone()), section(4, logs.clone())]),
            ),
            ("age", Tag::Long(1_793_576_311_368)),
        ]),
    };
    let region = bobby.join("r.-1.0.mca");
    let mut file = RegionFile::open_writable(&region, true).unwrap();
    let record = ChunkRecord::from_nbt(
        &chunk(DATA_VERSION),
        Compression::Zlib,
        0,
        Limits::default(),
    );
    file.write(ChunkPos::new(-2, 5), &record.unwrap()).unwrap();
    let record = ChunkRecord::from_nbt(
        &chunk(DATA_VERSION - 1),
        Compression::Zlib,
        0,
        Limits::default(),
    );
    file.write(ChunkPos::new(-1, 5), &record.unwrap()).unwrap();
    drop(file);
    let before = fs::read(&region).unwrap();
    let registry = assets.registry;
    let decode: DecodeChunk =
        Box::new(move |root| decode_lit(root, registry, &biomes, ColumnLayout::VANILLA, true));
    let terrain = ServerTerrain::with_bobby(own.clone(), Some(Bobby::new(bobby, decode, &facts)));
    let mut upright = registry
        .state(registry.default_state("minecraft:oak_log").unwrap())
        .unwrap()
        .clone();
    upright.properties.insert("axis".into(), "y".into());
    let upright = registry.id(&upright).unwrap();
    let stone = registry.default_state("minecraft:stone").unwrap();
    let columns = terrain.chunk(-2, 5).expect("Bobby's chunk");
    for column in columns.iter() {
        let surface = column.surface.expect("a surface");
        assert_eq!((surface.height, surface.state), (65, upright));
        assert!(
            column
                .spans
                .as_slice()
                .iter()
                .any(|span| span.state == stone)
        );
    }
    // Another game version's chunk is none.
    assert_eq!(terrain.chunk(-1, 5), None);
    drop(terrain);
    assert_eq!(
        fs::read(&region).unwrap(),
        before,
        "Bobby's folder is only read"
    );
    assert_eq!(ServerTerrain::new(own).chunk(-2, 5), Some(columns));
    let _ = fs::remove_dir_all(&dir);
}
/// A cache Bobby really wrote (`HORIZON_TEST_BOBBY`, its `.bobby`: the
/// fork's bobby-observe run, three local servers): found as the client
/// would find it, and every chunk of its Overworld folders decodes.
#[test]
fn far_terrain_reads_a_real_bobby_cache() {
    use crate::server_terrain::{Bobby, DecodeChunk, ServerTerrain, bobby_level_dir};
    let (Some(resources), Some(cache)) = (
        std::env::var_os("HORIZON_TEST_RESOURCES"),
        std::env::var_os("HORIZON_TEST_BOBBY"),
    ) else {
        return;
    };
    let resources = PathBuf::from(resources);
    let cache = PathBuf::from(cache);
    let assets = crate::assets::Assets::load(&resources).expect("assets");
    let biomes = Arc::new(
        Biomes::load(&horizon_assets::resource_source::ResourceRoot::directory(
            resources.to_path_buf(),
        ))
        .expect("biomes"),
    );
    let facts = crate::blocks::BlockFacts::load(&resources, assets.registry).expect("facts");
    let registry = assets.registry;
    let levels = [
        ("127.0.0.1", -9030034075591913594),
        ("127.0.0.1", 4696776658964135993),
        ("localhost:25566", -7181178862026813658),
    ];
    for (address, seed) in levels {
        let dir = bobby_level_dir(&cache, address, seed, "minecraft:overworld")
            .unwrap_or_else(|| panic!("{address} {seed}: no folder"));
        let mut chunks = Vec::new();
        for entry in fs::read_dir(&dir).unwrap().flatten() {
            if entry.path().extension().is_some_and(|e| e == "mca") {
                let file = RegionFile::open(entry.path()).unwrap();
                chunks.extend(file.chunks().map(|(pos, _)| pos));
            }
        }
        assert!(
            chunks.len() > 100,
            "{address} {seed}: {} chunks",
            chunks.len()
        );
        let own =
            std::env::temp_dir().join(format!("horizon-real-bobby-{seed}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&own);
        let biomes = biomes.clone();
        let decode: DecodeChunk =
            Box::new(move |root| decode_lit(root, registry, &biomes, ColumnLayout::VANILLA, true));
        let terrain = ServerTerrain::with_bobby(own.clone(), Some(Bobby::new(dir, decode, &facts)));
        let read = chunks
            .iter()
            .filter(|pos| {
                terrain
                    .chunk(pos.x, pos.z)
                    .is_some_and(|columns| columns.iter().all(|c| c.surface.is_some()))
            })
            .count();
        assert_eq!(
            read,
            chunks.len(),
            "{address} {seed}: chunks read with a surface"
        );
        let _ = fs::remove_dir_all(&own);
    }
}
