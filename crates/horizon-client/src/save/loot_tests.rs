use super::*;

pub(super) fn fixture() -> (BlockRegistry, Biomes) {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "horizon-opened-loot-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&root).unwrap();
    let maps = root.join("assets/minecraft/textures/colormap");
    fs::create_dir_all(&maps).unwrap();
    // Original one-pixel fixtures: chunk persistence only needs biome names.
    for name in ["grass", "foliage", "dry_foliage"] {
        let file = fs::File::create(maps.join(format!("{name}.png"))).unwrap();
        let mut encoder = png::Encoder::new(file, 1, 1);
        encoder.set_color(png::ColorType::Rgb);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&[0, 0, 0])
            .unwrap();
    }
    let folder = root.join("data/minecraft/worldgen/biome");
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join("plains.json"), "{}").unwrap();
    let biomes = Biomes::load(&horizon_assets::resource_source::ResourceRoot::directory(
        root.to_path_buf(),
    ))
    .unwrap();
    fs::remove_dir_all(&root).unwrap();
    let registry = BlockRegistry::from_nbt(&Tag::compound([
        ("DataVersion", Tag::Int(DATA_VERSION)),
        (
            "states",
            Tag::List(vec![
                Tag::compound([("id", string("minecraft:air"))]),
                Tag::compound([("id", string("minecraft:chest"))]),
            ]),
        ),
        (
            "defaults",
            Tag::compound([
                ("minecraft:air", Tag::Int(0)),
                ("minecraft:chest", Tag::Int(1)),
            ]),
        ),
    ]))
    .unwrap();
    (registry, biomes)
}

fn reload_loot_case(
    registry: &BlockRegistry,
    biomes: &Biomes,
    table: &str,
    seed: i64,
    mode: u8,
) -> Chunk {
    let pos = (1, 64, 2);
    let mut slots = vec![None; 27];

    let source = Tag::compound([
        ("id", string("minecraft:chest")),
        ("x", Tag::Int(pos.0)),
        ("y", Tag::Int(pos.1)),
        ("z", Tag::Int(pos.2)),
        ("LootTable", string(table)),
        ("LootTableSeed", Tag::Long(seed)),
        ("CustomName", string("{\"text\":\"Keep me\"}")),
    ]);
    // A loaded chest adopts its external name before the loot transition.
    let custom_name = source
        .get("CustomName")
        .and_then(crate::block_entity_nbt::component)
        .expect("fixture's admitted external name");
    let mut extras = ChunkExtras {
        block_entity_sources: vec![(pos, source)],
        pending_loot: [(
            pos,
            PendingContainerLoot {
                table: string(table),
                seed: Some(Tag::Long(seed)),
            },
        )]
        .into(),
        ..Default::default()
    };
    // Exactly the production transition used when the player opens the chest.
    if mode != 0 {
        extras.pending_loot.remove(&pos);
        slots.fill(None);
        if mode == 2 {
            slots[0] = Some(crate::items::ItemStack::new("minecraft:gold_ingot", 1));
        }
    }
    let mut chunk = Chunk::empty(0, 0);
    chunk.set(pos.0, pos.1, pos.2, 1);
    chunk
        .block_entities
        .push((pos, crate::containers::BlockEntity::Chest(slots)));
    chunk.block_names.push((pos, custom_name));
    chunk.saved = Some(Arc::new(extras));
    let encoded = encode(&chunk, registry, biomes);
    let bytes = nbt::write_named(&encoded, Limits::default()).unwrap();
    let (encoded, used) = nbt::read_named(&bytes, Limits::default()).unwrap();
    assert_eq!(used, bytes.len());
    decode(&encoded, registry, biomes).unwrap()
}

#[test]
fn emptied_opened_loot_stays_empty_after_chunk_save_and_reload() {
    let (registry, biomes) = fixture();
    let loaded = reload_loot_case(&registry, &biomes, "minecraft:chests/ruined_portal", 42, 1);
    let source = &loaded.saved.as_ref().unwrap().block_entity_sources[0].1;
    assert!(
        source.get("LootTable").is_none(),
        "opened chest keeps a rerollable loot table: {source:?}"
    );
    assert!(source.get("LootTableSeed").is_none());
    assert_eq!(
        text(source.get("CustomName")).as_deref(),
        Some("{\"text\":\"Keep me\"}")
    );
    let crate::containers::BlockEntity::Chest(slots) = &loaded.block_entities[0].1 else {
        panic!("chest");
    };
    assert!(slots.iter().all(Option::is_none));
}

#[test]
fn unopened_loot_tags_round_trip_exactly() {
    let (registry, biomes) = fixture();
    let pos = (1, 64, 2);
    // (seed read, seed saved): trySaveLootTable leaves out a seed that reads as 0, and
    // DoubleTag.longValue floors -0.5 to -1.
    let minus_half = Tag::Double((-0.5_f64).to_bits());
    for (seed, saved) in [
        (Some(Tag::Long(37)), Some(Tag::Long(37))),
        (None, None),
        (Some(Tag::Long(0)), None),
        (Some(minus_half.clone()), Some(minus_half)),
    ] {
        let pending = PendingContainerLoot {
            table: string("minecraft:chests/simple_dungeon"),
            seed,
        };
        let expected = |p: &PendingContainerLoot| {
            let mut map = Compound::new();
            map.insert("LootTable".into(), p.table.clone());
            if let Some(seed) = &p.seed {
                map.insert("LootTableSeed".into(), seed.clone());
            }
            nbt::write_named(
                &NamedTag {
                    name: "".into(),
                    tag: Tag::Compound(map),
                },
                Limits::default(),
            )
            .unwrap()
        };
        let bytes = expected(&PendingContainerLoot {
            table: pending.table.clone(),
            seed: saved,
        });
        let mut chunk = Chunk::empty(0, 0);
        chunk.set(1, 64, 2, 1);
        chunk
            .block_entities
            .push((pos, crate::containers::BlockEntity::Chest(vec![None; 27])));
        chunk.saved = Some(Arc::new(ChunkExtras {
            pending_loot: [(pos, pending)].into(),
            ..Default::default()
        }));
        for _ in 0..2 {
            let encoded = encode(&chunk, &registry, &biomes);
            let disk = nbt::write_named(&encoded, Limits::default()).unwrap();
            let (encoded, _) = nbt::read_named(&disk, Limits::default()).unwrap();
            let tag = &match encoded.tag.get("block_entities").unwrap() {
                Tag::List(tags) => tags,
                _ => panic!(),
            }[0];
            assert_eq!(
                expected(&PendingContainerLoot::from_tag(tag).unwrap()),
                bytes
            );
            assert!(tag.get("Items").is_none());
            chunk = decode(&encoded, &registry, &biomes).unwrap();
            assert!(
                chunk.block_entities[0]
                    .1
                    .slots()
                    .iter()
                    .all(Option::is_none)
            );
            let extras = chunk.saved.as_ref().unwrap();
            assert_eq!(expected(extras.pending_loot.get(&pos).unwrap()), bytes);
            assert!(extras.block_entity_sources[0].1.get("LootTable").is_none());
        }
    }
}

#[test]
#[ignore = "run by tools/test_opened_loot_parity.py"]
fn opened_loot_parity_fixture() {
    use std::io::Write;
    let (registry, biomes) = fixture();
    let input = fs::read_to_string(std::env::var("HORIZON_OPENED_LOOT_INPUT").unwrap()).unwrap();
    let mut output =
        fs::File::create(std::env::var("HORIZON_OPENED_LOOT_OUTPUT").unwrap()).unwrap();
    for line in input.lines() {
        let fields: Vec<_> = line.split_whitespace().collect();
        let mode = fields[2].parse().unwrap();
        let chunk = reload_loot_case(
            &registry,
            &biomes,
            fields[0],
            fields[1].parse().unwrap(),
            mode,
        );
        let source = encode(&chunk, &registry, &biomes);
        let Tag::List(tags) = source.tag.get("block_entities").unwrap() else {
            panic!()
        };
        let source = &tags[0];
        let crate::containers::BlockEntity::Chest(slots) = &chunk.block_entities[0].1 else {
            panic!("chest");
        };
        let count = if mode == 0 {
            "unopened".into()
        } else {
            slots
                .iter()
                .flatten()
                .map(|s| s.count)
                .sum::<u32>()
                .to_string()
        };
        writeln!(
            output,
            "{} {} {count}",
            u8::from(source.get("LootTable").is_some()),
            u8::from(source.get("LootTableSeed").is_some())
        )
        .unwrap();
    }
}

#[test]
fn eager_save_edits_reach_disk_during_ticks_and_again_after_cooldown() {
    use crate::world::ModifiedChunks;
    use std::time::{Duration, Instant};
    let (registry, biomes) = fixture();
    let registry = Box::leak(Box::new(registry));
    let biomes = Arc::new(biomes);
    let dir = std::env::temp_dir().join(format!("horizon-eager-edits-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let save = SaveDir::new(dir.clone(), LevelShape::OVERWORLD);
    let mut chunk = Chunk::empty(0, 0);
    let mut dirty = ModifiedChunks::default();
    let now = Instant::now();
    for tick in [0, 1, 199, 200] {
        if tick <= 1 {
            chunk.set(1, 64, 2, if tick == 0 { 1 } else { 0 });
            dirty.insert((0, 0));
        }
        dirty.save_eagerly(
            now + Duration::from_millis(tick * 50),
            || save.active_chunk_writes() < 128,
            |_| true,
            |_| {
                save.save_later(&[Arc::new(chunk.clone())], registry, &biomes)
                    .unwrap();
                true
            },
        );
        save.flush().unwrap(); // only a barrier: no quit/autosave submission
        let reopened = SaveDir::new(dir.clone(), LevelShape::OVERWORLD);
        assert_eq!(
            reopened
                .load(0, 0, registry, &biomes)
                .unwrap()
                .get(1, 64, 2),
            if tick < 200 { 1 } else { 0 }
        );
    }
    drop(save);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn container_limit_survives_chunk_save_and_reload() {
    let (registry, biomes) = fixture();
    let pos = (1, 64, 2);
    let mut chunk = Chunk::empty(0, 0);
    chunk.set(1, 64, 2, 1);
    chunk
        .block_entities
        .push((pos, crate::containers::BlockEntity::Chest(vec![None; 27])));
    chunk.saved = Some(Arc::new(ChunkExtras {
        block_entity_sources: vec![(pos, Tag::compound([("horizon:stack_limit", Tag::Int(10))]))],
        ..Default::default()
    }));
    let encoded = encode(&chunk, &registry, &biomes);
    let loaded = decode(&encoded, &registry, &biomes).unwrap();
    assert_eq!(
        loaded.saved.as_ref().unwrap().block_entity_sources[0]
            .1
            .get("horizon:stack_limit"),
        Some(&Tag::Int(10))
    );
}

#[test]
fn unmodified_container_nbt_has_no_stack_limit() {
    let (registry, biomes) = fixture();
    let mut chunk = Chunk::empty(0, 0);
    chunk.set(1, 64, 2, 1);
    chunk.block_entities.push((
        (1, 64, 2),
        crate::containers::BlockEntity::Chest(vec![None; 27]),
    ));
    let loaded = decode(&encode(&chunk, &registry, &biomes), &registry, &biomes).unwrap();
    assert_eq!(loaded.saved.as_ref().unwrap().block_entity_sources.len(), 1);
    for (_, tag) in &loaded.saved.as_ref().unwrap().block_entity_sources {
        assert!(tag.get("horizon:stack_limit").is_none());
    }
}

#[test]
#[ignore = "daily"]
fn generated_structure_chest_stays_pending_after_save_and_reload() {
    use crate::{
        streaming::{ChunkGenerator, Proto},
        vanillagen::{TerrainResources, VanillaTerrain},
    };
    let root = PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    let assets = crate::assets::Assets::load(&root).unwrap();
    let biomes = Biomes::load(&horizon_assets::resource_source::ResourceRoot::directory(
        root.clone(),
    ))
    .unwrap();
    let items = crate::test_resources::item_data(
        &root,
        horizon_world::native_load_context::DataLoadOrigin::ServerDataLoad,
    )
    .unwrap();
    // Existing generation_saves::saved_proto_block_entities_keep_their_id fixture:
    // seed 12345, cold ocean ruin centered on chunk (43, 21).
    let generator = VanillaTerrain::for_dimension(
        TerrainResources {
            data: None,
            dir: &root.join("worldgen"),
            registry: assets.registry,
            biomes: &biomes,
            item_data: &items,
        },
        12345,
        "minecraft:overworld",
    )
    .unwrap();
    let mut worker = generator.worker();
    let mut region: [Proto; 9] = std::array::from_fn(|i| {
        generator.proto(&mut worker, 43 + i as i32 % 3 - 1, 21 + i as i32 / 3 - 1)
    });
    generator.decorate(&mut worker, &mut region);
    let chunk = generator.finish(region.into_iter().nth(4).unwrap());
    const CHEST: (i32, i32, i32) = (692, 60, 348);
    const TABLE: &str = "minecraft:chests/underwater_ruin_small";
    const SEED: i64 = 8294534313461660897;
    let expected = PendingContainerLoot {
        table: string(TABLE),
        seed: Some(Tag::Long(SEED)),
    };
    assert_eq!(
        chunk.saved.as_ref().unwrap().pending_loot.get(&CHEST),
        Some(&expected)
    );
    assert!(
        chunk
            .block_entities
            .iter()
            .find(|(p, _)| *p == CHEST)
            .unwrap()
            .1
            .slots()
            .iter()
            .all(Option::is_none)
    );
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "generated-pending-loot-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&dir).unwrap();
    let save = SaveDir::new(dir.clone(), LevelShape::OVERWORLD);
    save.save(&[Arc::new(chunk)], assets.registry, &biomes)
        .unwrap();
    save.flush().unwrap();
    let reopened = SaveDir::new(dir.clone(), LevelShape::OVERWORLD);
    let reloaded = reopened.load(43, 21, assets.registry, &biomes).unwrap();
    assert_eq!(
        reloaded.saved.as_ref().unwrap().pending_loot.get(&CHEST),
        Some(&expected)
    );
    assert!(
        reloaded
            .block_entities
            .iter()
            .find(|(p, _)| *p == CHEST)
            .unwrap()
            .1
            .slots()
            .iter()
            .all(Option::is_none)
    );
    let saved = encode(&reloaded, assets.registry, &biomes);
    let Tag::List(entities) = saved.tag.get("block_entities").unwrap() else {
        panic!()
    };
    let chest = entities
        .iter()
        .find(|e| {
            number(e.get("x")) == Some(692)
                && number(e.get("y")) == Some(60)
                && number(e.get("z")) == Some(348)
        })
        .unwrap();
    assert_eq!(chest.get("LootTable"), Some(&string(TABLE)));
    assert_eq!(chest.get("LootTableSeed"), Some(&Tag::Long(SEED)));
    assert!(chest.get("Items").is_none());
    drop(reopened);
    drop(save);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_loot_table_that_is_not_an_identifier_is_dropped_with_its_seed() {
    // RandomizableContainer.tryLoadLootTable reads LootTable with the identifier codec: a string
    // that does not parse leaves no table, and with no table the seed is not saved either.
    let (registry, biomes) = fixture();
    let pos = (1, 64, 2);
    let mut chunk = Chunk::empty(0, 0);
    chunk.set(pos.0, pos.1, pos.2, 1);
    chunk
        .block_entities
        .push((pos, crate::containers::BlockEntity::Chest(vec![None; 27])));
    let mut encoded = encode(&chunk, &registry, &biomes);
    let Tag::Compound(root) = &mut encoded.tag else {
        panic!("chunk root");
    };
    let Some(Tag::List(entities)) = root.get_mut(&"block_entities".into()) else {
        panic!("no block entities");
    };
    let Tag::Compound(chest) = &mut entities[0] else {
        panic!("chest");
    };
    chest.insert("LootTable".into(), string("Bad Name!"));
    chest.insert("LootTableSeed".into(), Tag::Long(73));
    let loaded = decode(&encoded, &registry, &biomes).unwrap();
    assert!(
        loaded
            .saved
            .as_ref()
            .is_none_or(|extras| extras.pending_loot.is_empty())
    );
    let saved = encode(&loaded, &registry, &biomes);
    let Some(Tag::List(entities)) = saved.tag.get("block_entities") else {
        panic!("no block entities");
    };
    assert_eq!(
        (
            entities[0].get("LootTable"),
            entities[0].get("LootTableSeed")
        ),
        (None, None)
    );
}

#[test]
fn block_entity_components_stay_with_the_model_through_chunk_nbt() {
    let (registry, biomes) = fixture();
    let pos = (1, 64, 2);
    let chest = registry.default_state("minecraft:chest").unwrap();
    let mut chunk = Chunk::empty(0, 0);
    chunk.set(pos.0, pos.1, pos.2, chest.0);
    let mut world = crate::world::World::new(registry.default_state("minecraft:air").unwrap(), 0);
    world.chunks.insert((0, 0), Arc::new(chunk));
    let read = |input: &str| horizon_core::nbt::parse_snbt(input).unwrap().0;
    let mut raw = read(
        r#"{id:"minecraft:chest",x:1,y:64,z:2,CustomName:"Implicit",components:{custom_name:"Explicit",custom_data:{a:1b},rarity:"made_up",unregistered:{}}}"#,
    );
    let expected = read(r#"{"minecraft:custom_name":"Explicit","minecraft:custom_data":{a:1b}}"#);
    let registry_for_components = horizon_client_core::registries::Registries {
        persistent_components: [
            "minecraft:custom_name",
            "minecraft:custom_data",
            "minecraft:rarity",
        ]
        .map(str::to_owned)
        .to_vec(),
        ..Default::default()
    };
    normalize_block_entity_components(&mut raw, &registry_for_components);
    let (_, entity) = decode_block_entity(&raw).unwrap();
    world.block_entities.insert(pos, entity);
    world
        .block_names
        .insert(pos, Tag::String("Implicit".into()));
    assert!(world.set_block_entity_metadata(pos, &raw));

    let entity = world.block_entities.get(&pos).unwrap();
    let mut queried = block_entity_data(pos, entity, "minecraft:chest", None);
    write_stored_block_entity_components(&mut queried, entity, world.block_entity_source(pos));
    assert_eq!(queried.get("components"), Some(&expected));
    let chunk = chunks_with_block_entities(
        [world.chunks.get(&(0, 0)).unwrap()],
        &world.block_entities,
        &world.block_names,
    )
    .remove(0);
    let saved = encode(&chunk, &registry, &biomes);
    let loaded = decode(&saved, &registry, &biomes).unwrap();
    assert_eq!(loaded.block_names, [(pos, Tag::String("Implicit".into()))]);
    let source = &loaded.saved.as_ref().unwrap().block_entity_sources[0].1;
    assert_eq!(source.get("components"), Some(&expected));
}

#[test]
fn stored_container_slots_resolve_item_ids_on_chunk_load() {
    let (registry, biomes) = fixture();
    let pos = (1, 64, 2);
    let chest = registry.default_state("minecraft:chest").unwrap();
    let mut chunk = Chunk::empty(0, 0);
    chunk.set(pos.0, pos.1, pos.2, chest.0);
    let mut saved = encode(&chunk, &registry, &biomes);
    let read = |input: &str| horizon_core::nbt::parse_snbt(input).unwrap().0;
    let Tag::Compound(fields) = &mut saved.tag else {
        panic!("saved chunk must be a compound");
    };
    fields.insert(
        "block_entities".into(),
        Tag::List(vec![read(
            r#"{id:"minecraft:chest",x:1,y:64,z:2,components:{container:[
                {slot:0,item:{id:stone}},
                {slot:0,item:{id:"minecraft:noitem"}},
                {slot:1,item:{id:"minecraft:noitem"}},
                {slot:2,item:{id:"pack:gem"}}
            ]}}"#,
        )]),
    );
    let item_registry =
        horizon_client_core::registries::Registries::from_export(&serde_json::json!({
            "persistent_components": ["minecraft:container"],
            "by_registry": {"minecraft:item": {"ids": ["minecraft:stone"]}}
        }));
    let tables = SaveTables::for_heightmaps(crate::assets::Properties::for_tests(&[]))
        .with_component_registry(Arc::new(item_registry), ["pack:gem".to_owned()]);
    let loaded = decode_with(
        &saved,
        &registry,
        &biomes,
        Some(&tables),
        ColumnLayout::VANILLA,
    )
    .expect("load chunk");
    let [(loaded_pos, entity)] = loaded.block_entities.as_slice() else {
        panic!("expected one loaded chest");
    };
    assert_eq!(*loaded_pos, pos);
    let source = &loaded.saved.as_ref().unwrap().block_entity_sources[0].1;
    let mut queried = block_entity_data(pos, entity, "minecraft:chest", None);
    write_stored_block_entity_components(&mut queried, entity, Some(source));

    let expected = read(
        r#"{"minecraft:container":[{slot:0,item:{id:"minecraft:stone"}},{slot:2,item:{id:"pack:gem"}}]}"#,
    );
    assert_eq!(queried.get("components"), Some(&expected));
}

#[test]
fn stored_lock_resolves_item_tags_and_decodes_subpredicates_on_command_and_reload() {
    let (registry, biomes) = fixture();
    let pos = (1, 64, 2);
    let chest = registry.default_state("minecraft:chest").unwrap();
    let mut chunk = Chunk::empty(0, 0);
    chunk.set(pos.0, pos.1, pos.2, chest.0);
    let mut saved = encode(&chunk, &registry, &biomes);
    let read = |input: &str| horizon_core::nbt::parse_snbt(input).unwrap().0;
    let item_registry = Arc::new(horizon_client_core::registries::Registries::from_export(
        &serde_json::json!({
            "persistent_components": ["minecraft:lock"],
            "by_registry": {
                "minecraft:item": {"ids": ["minecraft:stone"], "tags": ["minecraft:logs"]},
                "minecraft:data_component_type": {"ids": ["minecraft:lock", "minecraft:rarity"]}
            }
        }),
    ));
    let tables = SaveTables::for_heightmaps(crate::assets::Properties::for_tests(&[]))
        .with_component_registry(Arc::clone(&item_registry), []);
    tables.install_component_item_tags(item_registry.tags("minecraft:item"));

    // Expected values from ItemPredicate and DataComponentPredicate in vanilla 26.3.
    for (lock, expected) in [
        (
            r#"{predicates:{written_book_content:{pages:{contains:[{nbt:"a",source:"block",block:"foo"},{nbt:"a",block:"0 64 -2"},{nbt:"a",block:"~ ~1.5 ~"},{nbt:"a",block:"^ ^.5 ^-2"},{nbt:"a",block:"^ ~ ^"},{nbt:"a",block:"0 64 2 extra"}]}}}}"#,
            r#"{"minecraft:lock":{predicates:{"minecraft:written_book_content":{pages:{contains:[{block:"0 64 -2",nbt:"a"},{block:"~ ~1.5 ~",nbt:"a"},{block:"^ ^.5 ^-2",nbt:"a"}]}}}}}"#,
        ),
        (
            r#"{predicates:{written_book_content:{pages:{contains:[{nbt:"a",source:"entity",entity:"@q"},{nbt:"a",entity:"@e[limit=]"},{nbt:"a",entity:"@s"},{nbt:"a",entity:"@e[type=pig,limit=1]"},{nbt:"a",entity:"Steve"},{nbt:"a",storage:"example:test"},{nbt:"a",storage:"unqualified"},{nbt:"a",storage:"invalid storage"}]}}}}"#,
            r#"{"minecraft:lock":{predicates:{"minecraft:written_book_content":{pages:{contains:[{entity:"@s",nbt:"a"},{entity:"@e[type=pig,limit=1]",nbt:"a"},{entity:"Steve",nbt:"a"},{nbt:"a",storage:"example:test"},{nbt:"a",storage:"minecraft:unqualified"}]}}}}}"#,
        ),
        (
            r#"{predicates:{written_book_content:{pages:{contains:[{nbt:"a",source:"block",block:"foo",storage:"valid"},{nbt:"a",source:"storage",block:"0 0 0",storage:"valid"},{nbt:"a",source:"bad",storage:"valid"},{nbt:"a",source:7,storage:"valid"},{nbt:"a",block:"0 0 0",entity:"@s",storage:"valid"},{nbt:"a",block:"foo",entity:"@q",storage:"valid"},{nbt:"a",block:"0 0 0",storage:"valid"},{nbt:"a",entity:"@s",storage:"valid"}]}}}}"#,
            r#"{"minecraft:lock":{predicates:{"minecraft:written_book_content":{pages:{contains:[{nbt:"a",storage:"minecraft:valid"},{entity:"@s",nbt:"a"},{nbt:"a",storage:"minecraft:valid"},{block:"0 0 0",nbt:"a"},{entity:"@s",nbt:"a"}]}}}}}"#,
        ),
        (
            r#"{predicates:{written_book_content:{pages:{contains:[{text:7},{text:"hi",bold:"bad"}]}}}}"#,
            r#"{"minecraft:lock":{predicates:{"minecraft:written_book_content":{pages:{contains:["hi"]}}}}}"#,
        ),
        (
            r#"{predicates:{written_book_content:{pages:{contains:[{}, {text:7},{text:"hi",bold:"bad"},{translate:7},{translate:"key",fallback:7},{keybind:"abc"},[],["one",7,"three"],{type:"text",text:"typed"},{type:"bad",text:"a"},{text:"yes",extra:[7,"no"]}]}}}}"#,
            r#"{"minecraft:lock":{predicates:{"minecraft:written_book_content":{pages:{contains:["hi",{translate:"key"},{keybind:"abc"},{extra:["three"],text:"one"},"typed",{extra:["no"],text:"yes"}]}}}}}"#,
        ),
        (
            r##"{predicates:{written_book_content:{pages:{contains:[{text:"style",shadow_color:[1.0,0.0,0.0,1.0]},{text:"style",click_event:{action:"bad"}},{text:"style",click_event:{action:"run_command",command:7}},{text:"style",hover_event:{action:"show_text",value:{text:"tip",bold:"bad"}}},{text:"style",shadow_color:12.3f},{text:"style",color:"#abc",font:"uniform"}]}}}}"##,
            r##"{"minecraft:lock":{predicates:{"minecraft:written_book_content":{pages:{contains:[{shadow_color:-65536,text:"style"},"style","style",{hover_event:{action:"show_text",value:"tip"},text:"style"},{shadow_color:12,text:"style"},{color:"#000ABC",font:"minecraft:uniform",text:"style"}]}}}}}"##,
        ),
        (
            r#"{predicates:{enchantments:[{levels:"bad"}],stored_enchantments:[{enchantments:"sharpness",levels:2}]}}"#,
            r#"{"minecraft:lock":{predicates:{"minecraft:enchantments":[{}],"minecraft:stored_enchantments":[{enchantments:"minecraft:sharpness",levels:2}]}}}"#,
        ),
        (
            r##"{predicates:{enchantments:[{levels:"bad"},{enchantments:["sharpness"],levels:{min:2,max:2}},{enchantments:"#minecraft:exclusive_set/damage",levels:{min:1,max:"bad"}},{enchantments:2,levels:2}],stored_enchantments:[{levels:"bad"}],damage:{damage:3}}}"##,
            r##"{"minecraft:lock":{predicates:{"minecraft:damage":{damage:3},"minecraft:enchantments":[{},{enchantments:"minecraft:sharpness",levels:2},{enchantments:"#minecraft:exclusive_set/damage",levels:{min:1}},{levels:2}],"minecraft:stored_enchantments":[{}]}}}"##,
        ),
        (
            r##"{predicates:{container:{items:{contains:[{items:2,count:2},7,{items:"#logs",predicates:{enchantments:[{levels:"bad"}]}}],count:[{test:{items:"stone"},count:2},{test:{items:"stone"},count:"bad"},{count:2},7],size:{min:1,max:"bad"},other:3}},bundle_contents:{items:7}}}"##,
            r##"{"minecraft:lock":{predicates:{"minecraft:bundle_contents":{},"minecraft:container":{items:{contains:[{count:2},{items:"#minecraft:logs",predicates:{"minecraft:enchantments":[{}]}}],count:[{count:2,test:{items:"minecraft:stone"}}],size:{min:1}}}}}}"##,
        ),
        (
            r#"{predicates:{writable_book_content:{pages:{contains:["abc",7],count:[{test:"one",count:{min:1}}]}},written_book_content:{pages:{contains:[{text:"paragraph"},7]},author:7,title:"Good",generation:{min:1,max:1},resolved:-1}}}"#,
            r#"{"minecraft:lock":{predicates:{"minecraft:writable_book_content":{pages:{contains:["abc"],count:[{count:{min:1},test:"one"}]}},"minecraft:written_book_content":{generation:1,pages:{contains:["paragraph"]},resolved:1b,title:"Good"}}}}"#,
        ),
        (
            r##"{items:"#minecraft:no_such_item_tag",count:1.9f,predicates:{custom_data:7,damage:{durability:1,unrecognized:3},unknown:{x:1}}}"##,
            r#"{"minecraft:lock":{count:1,predicates:{"minecraft:damage":{durability:1}}}}"#,
        ),
        (
            r##"{items:"#logs",predicates:{custom_data:{key:1b},rarity:{}}}"##,
            r##"{"minecraft:lock":{items:"#minecraft:logs",predicates:{"minecraft:custom_data":"{key:1b}","minecraft:rarity":{}}}}"##,
        ),
        (
            r#"{predicates:{enchantments:[7,{}],stored_enchantments:[I;2],container:{bad:1},potion_contents:7,"villager/variant":"plains"}}"#,
            r#"{"minecraft:lock":{predicates:{"minecraft:container":{},"minecraft:enchantments":[{}],"minecraft:stored_enchantments":[],"minecraft:villager/variant":"minecraft:plains"}}}"#,
        ),
    ] {
        let raw = read(&format!(
            r#"{{id:"minecraft:chest",x:1,y:64,z:2,components:{{lock:{lock}}}}}"#
        ));
        let expected = read(expected);

        let mut command = raw.clone();
        normalize_block_entity_components(&mut command, &item_registry);
        assert_eq!(command.get("components"), Some(&expected));

        let Tag::Compound(fields) = &mut saved.tag else {
            panic!("saved chunk must be a compound");
        };
        fields.insert("block_entities".into(), Tag::List(vec![raw]));
        let loaded = decode_with(
            &saved,
            &registry,
            &biomes,
            Some(&tables),
            ColumnLayout::VANILLA,
        )
        .expect("load chunk");
        let [(loaded_pos, entity)] = loaded.block_entities.as_slice() else {
            panic!("expected one loaded chest");
        };
        assert_eq!(*loaded_pos, pos);
        let source = &loaded.saved.as_ref().unwrap().block_entity_sources[0].1;
        let queried =
            full_block_entity_data(pos, entity, "minecraft:chest", Some(source), None, None);
        assert_eq!(queried.get("components"), Some(&expected));
    }
}
