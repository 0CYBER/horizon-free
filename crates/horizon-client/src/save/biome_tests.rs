use super::*;

#[test]
#[ignore = "daily"]
fn a_biome_the_world_lacks_is_saved_back_unchanged() {
    let Some(root) = std::env::var_os("HORIZON_TEST_RESOURCES") else {
        return;
    };
    let root = PathBuf::from(root);
    let assets = crate::assets::Assets::load(&root).expect("assets");
    let load = || {
        let biomes = Biomes::load(&horizon_assets::resource_source::ResourceRoot::directory(
            root.clone(),
        ))
        .expect("biomes");
        // Force the saved palette into global storage with IDs above 255.
        for i in 0..260 {
            biomes.saved_id(&format!("gone_mod:unused_{i}"));
        }
        biomes
    };
    let palette = |saved: &NamedTag| -> Vec<String> {
        let Some(Tag::List(sections)) = saved.tag.get("sections") else {
            panic!("sections");
        };
        sections
            .iter()
            .filter_map(|s| match s.get("biomes")?.get("palette")? {
                Tag::List(p) => Some(p.clone()),
                _ => None,
            })
            .flatten()
            .filter_map(|t| match t {
                Tag::String(s) => Some(s.to_string_lossy()),
                _ => None,
            })
            .collect()
    };
    // A world saved while a mod's biome was installed.
    let before = load();
    let mut chunk = Chunk::empty(0, 0);
    chunk.set_biome_cell(1, 20, 2, before.saved_id("gone_mod:highland"));
    for i in 0..9 {
        chunk.set_biome_cell(
            i % 4,
            20,
            i / 4,
            before.saved_id(&format!("gone_mod:cell_{i}")),
        );
    }
    let saved = encode(&chunk, assets.registry, &before);
    assert!(palette(&saved).contains(&"gone_mod:highland".to_owned()));
    // Reopened without that mod: the column plays as plains...
    let after = load();
    let loaded = decode(&saved, assets.registry, &after).expect("chunk");
    // Quart (1, 20, 2) above the bottom at -64: block (4, 16, 8).
    let id = loaded.biome(4, 16, 8);
    assert_eq!(after.name(id), "minecraft:plains");
    // ...and its next save still names the missing biome, not plains.
    let resaved = encode(&loaded, assets.registry, &after);
    let names = palette(&resaved);
    assert!(names.contains(&"gone_mod:highland".to_owned()), "{names:?}");
    assert!(!names.contains(&"minecraft:plains".to_owned()), "{names:?}");
    // Reopened again, still without the mod: the same name, still playing as plains.
    let again = load();
    let reloaded = decode(&resaved, assets.registry, &again).expect("chunk");
    let id = reloaded.biome(4, 16, 8);
    assert_eq!(again.name(id), "minecraft:plains");
    assert_eq!(again.saved_name(id), "gone_mod:highland");
    for i in 0..9 {
        let id = reloaded.biome((i % 4) * 4, 16, (i / 4) * 4);
        assert!(id > 255);
        assert_eq!(again.saved_name(id), format!("gone_mod:cell_{i}"));
    }
    assert_eq!(palette(&encode(&reloaded, assets.registry, &again)), names);
}

#[test]
#[ignore = "daily"]
fn vanilla_biome_palettes_are_byte_identical_after_reload() {
    let root = PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").expect("resources"));
    let assets = crate::assets::Assets::load(&root).expect("assets");
    let biomes = Biomes::load(&horizon_assets::resource_source::ResourceRoot::directory(
        root,
    ))
    .expect("biomes");
    let mut chunk = Chunk::empty(0, 0);
    for i in 0..64 {
        chunk.set_biome_cell(
            i % 4,
            20 + i / 16,
            (i / 4) % 4,
            biomes.id(if i % 2 == 0 {
                "minecraft:forest"
            } else {
                "minecraft:plains"
            }),
        );
    }
    let saved = encode(&chunk, assets.registry, &biomes);
    let loaded = decode(&saved, assets.registry, &biomes).expect("chunk");
    let resaved = encode(&loaded, assets.registry, &biomes);
    let palettes = |root: &NamedTag| {
        let Some(Tag::List(sections)) = root.tag.get("sections") else {
            panic!("sections")
        };
        sections
            .iter()
            .map(|section| {
                let tag = NamedTag {
                    name: "".into(),
                    tag: section.get("biomes").expect("biomes").clone(),
                };
                horizon_core::nbt::write_named(&tag, horizon_core::nbt::Limits::default())
                    .expect("NBT")
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(palettes(&saved), palettes(&resaved));
}

/// Every file under `root` with its bytes.
fn disk_bytes(root: &std::path::Path) -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
    fn walk(dir: &std::path::Path, found: &mut std::collections::BTreeMap<PathBuf, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, found);
            } else {
                found.insert(path.clone(), std::fs::read(&path).unwrap());
            }
        }
    }
    let mut found = std::collections::BTreeMap::new();
    walk(root, &mut found);
    found
}

#[test]
#[ignore = "daily"]
fn an_exhausted_biome_table_refuses_every_save_before_touching_the_world() {
    let (registry, biomes) = super::loot_tests::fixture();
    let registry: &'static BlockRegistry = Box::leak(Box::new(registry));
    let biomes = Arc::new(biomes);
    let root = std::env::temp_dir().join(format!("horizon-biome-exhausted-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("playerdata")).unwrap();
    std::fs::write(root.join("level.dat"), b"level bytes").unwrap();
    std::fs::write(root.join("playerdata/a.dat"), b"player bytes").unwrap();
    let save = SaveDir::new(root.clone(), LevelShape::OVERWORLD);
    save.save(&[Arc::new(Chunk::empty(0, 0))], registry, &biomes)
        .unwrap();
    let before = disk_bytes(&root);
    assert!(
        before
            .keys()
            .any(|path| path.starts_with(root.join("region"))),
        "the world already holds a region file"
    );
    // A world whose saved names fill every biome id: the next one has no place.
    let mut allocated = 0;
    while biomes.ensure_saveable().is_ok() {
        biomes.saved_id(&format!("gone_mod:biome_{allocated}"));
        allocated += 1;
        assert!(allocated < 70_000, "the biome table must run out of ids");
    }
    let refused = |error: Box<dyn std::error::Error>| {
        assert!(
            error
                .downcast_ref::<crate::biome::BiomeIdExhausted>()
                .is_some(),
            "{error}"
        );
        assert!(error.to_string().contains("65536"), "{error}");
    };
    refused(
        save.save(&[Arc::new(Chunk::empty(1, 0))], registry, &biomes)
            .unwrap_err(),
    );
    refused(
        save.save_later(&[Arc::new(Chunk::empty(1, 0))], registry, &biomes)
            .unwrap_err(),
    );
    let _ = save.flush();
    refused(save.save_protos(Vec::new(), registry, &biomes).unwrap_err());
    // A record that reaches the writer thread is refused too, without a file.
    let encoding = Arc::new(Encoding {
        registry,
        biomes: biomes.clone(),
        tables: None,
        shape: save.shape,
        section_palettes: None,
    });
    let record = Record::Chunk(Arc::new(Chunk::empty(1, 0)), encoding, None);
    match &compress_records(&[((1, 0), &record)], 0)[0] {
        Err(error) => assert!(error.to_string().contains("65536"), "{error}"),
        Ok(_) => panic!("an exhausted table compressed a chunk"),
    }
    assert_eq!(
        disk_bytes(&root),
        before,
        "no byte of the world changed after the refusals"
    );
    let _ = std::fs::remove_dir_all(&root);
}
