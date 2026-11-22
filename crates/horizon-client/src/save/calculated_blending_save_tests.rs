use super::*;
use horizon_world::worldgen::blending::BlendingData;

fn heights() -> [f32; 16] {
    let mut h = [f32::MAX; 16];
    h[12..].copy_from_slice(&[70.0, 70.0, 69.0, 68.0]);
    h
}
fn calculated(min: i32, max: i32) -> Arc<BlendingData> {
    Arc::new(BlendingData::unpack(min, max, Some(&heights())).unwrap())
}
/// A loaded old chunk: its raw `blending_data` kept in the loaded root.
fn old_chunk(x: i32, z: i32) -> Chunk {
    let mut chunk = Chunk::empty(x, z);
    let mut extras = crate::chunk::ChunkExtras::default();
    extras.root.insert(
        "blending_data".into(),
        Tag::compound([("min_section", Tag::Int(-4)), ("max_section", Tag::Int(20))]),
    );
    chunk.saved = Some(Arc::new(extras));
    chunk
}
fn saved_heights(save: &SaveDir, x: i32, z: i32) -> Option<Option<Vec<u32>>> {
    let root = save.read_record_raw("region", x, z)?;
    let data = root.tag.get("blending_data")?.clone();
    Some(match data.get("heights") {
        Some(Tag::List(list)) => Some(
            list.iter()
                .map(|v| match v {
                    Tag::Float(bits) => *bits,
                    other => panic!("{other:?}"),
                })
                .collect(),
        ),
        _ => None,
    })
}
fn resources() -> Option<(crate::assets::Assets, Arc<Biomes>)> {
    let root = PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES")?);
    let assets = crate::assets::Assets::load(&root).expect("assets");
    let biomes = Arc::new(
        Biomes::load(&horizon_assets::resource_source::ResourceRoot::directory(
            root.clone(),
        ))
        .expect("biomes"),
    );
    Some((assets, biomes))
}
fn folder(name: &str, assets: &crate::assets::Assets) -> (PathBuf, SaveDir) {
    let dir = std::env::temp_dir().join(format!("horizon-h077-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let save = SaveDir::new(dir.clone(), LevelShape::OVERWORLD);
    let _ = save.tables.set(Arc::new(SaveTables::new(assets)));
    (dir, save)
}
fn want() -> Vec<u32> {
    heights().map(f32::to_bits).to_vec()
}

/// The owner of a cache entry, as `SavedOldWorld` owns its cache.
struct Owner(Arc<BlendingData>);
/// A lookup that reads `owner`'s entry through a weak link, as the game's does.
fn hook(save: &SaveDir, at: (i32, i32), owner: &Arc<Owner>) {
    let weak = Arc::downgrade(owner);
    save.set_blending_lookup(Box::new(move |x, z| {
        if (x, z) == at {
            weak.upgrade().map(|o| o.0.clone())
        } else {
            None
        }
    }));
}

/// The real `save_later` path writes the calculated heights, and only for
/// the chunk the lookup has them for.
#[test]
fn save_later_writes_the_calculated_heights_of_a_hooked_folder() {
    let Some((assets, biomes)) = resources() else {
        return;
    };
    let (dir, save) = folder("later", &assets);
    let owner = Arc::new(Owner(calculated(-4, 20)));
    hook(&save, (12, 0), &owner);
    save.save_later(
        &[Arc::new(old_chunk(12, 0)), Arc::new(old_chunk(11, 0))],
        assets.registry,
        &biomes,
    )
    .unwrap();
    save.flush().unwrap();
    assert_eq!(saved_heights(&save, 12, 0), Some(Some(want())));
    // No calculation for (11, 0): its raw compound, without heights.
    assert_eq!(saved_heights(&save, 11, 0), Some(None));
    let _ = fs::remove_dir_all(&dir);
}

/// The owner is gone before the queued record is encoded: the record
/// keeps the snapshot taken at the request (a lookup at encoding would
/// find nothing).
#[test]
fn a_queued_record_keeps_its_snapshot_after_the_owner_is_dropped() {
    let Some((assets, biomes)) = resources() else {
        return;
    };
    let (dir, save) = folder("owner", &assets);
    let owner = Arc::new(Owner(calculated(-4, 20)));
    hook(&save, (12, 0), &owner);
    let encoding = Arc::new(Encoding {
        registry: assets.registry,
        biomes: biomes.clone(),
        tables: save.tables.get().cloned(),
        shape: save.shape,
        section_palettes: None,
    });
    // As `save_later` makes it: the snapshot taken at the request.
    let record = Record::Chunk(
        Arc::new(old_chunk(12, 0)),
        encoding,
        save.calculated_blending(12, 0),
    );
    drop(owner);
    assert!(
        save.calculated_blending(12, 0).is_none(),
        "the owner is gone"
    );
    let tag = record.tag().expect("a chunk record encodes");
    let heights = match tag.tag.get("blending_data").and_then(|b| b.get("heights")) {
        Some(Tag::List(list)) => list
            .iter()
            .map(|v| match v {
                Tag::Float(bits) => *bits,
                other => panic!("{other:?}"),
            })
            .collect::<Vec<u32>>(),
        other => panic!("heights {other:?}"),
    };
    assert_eq!(heights, want());
    let _ = fs::remove_dir_all(&dir);
}

/// The same coordinates in another dimension's folder (no lookup), and a
/// lookup whose section range differs from the saved record: the raw
/// compound stays.
#[test]
fn an_unhooked_folder_or_another_section_range_keeps_the_raw_compound() {
    let Some((assets, biomes)) = resources() else {
        return;
    };
    let (dir, nether) = folder("nether", &assets);
    let owner = Arc::new(Owner(calculated(-4, 20)));
    nether
        .save_later(&[Arc::new(old_chunk(12, 0))], assets.registry, &biomes)
        .unwrap();
    nether.flush().unwrap();
    assert_eq!(saved_heights(&nether, 12, 0), Some(None));
    let _ = fs::remove_dir_all(&dir);
    let (dir, other) = folder("range", &assets);
    let elsewhere = Arc::new(Owner(calculated(0, 16)));
    hook(&other, (12, 0), &elsewhere);
    other
        .save(&[Arc::new(old_chunk(12, 0))], assets.registry, &biomes)
        .unwrap();
    assert_eq!(saved_heights(&other, 12, 0), Some(None));
    drop(owner);
    let _ = fs::remove_dir_all(&dir);
}
