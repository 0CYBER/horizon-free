use super::*;
use horizon_core::nbt::NamedTag;

struct TempWorld(PathBuf);
impl TempWorld {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("horizon-wu-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
    fn folder(&self, dimension: &str, folder: &str) -> PathBuf {
        dimension_path(&self.0, dimension).join(folder)
    }
}
impl Drop for TempWorld {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn root(tag: Tag) -> NamedTag {
    NamedTag {
        name: "".into(),
        tag,
    }
}

fn write_chunks(file: &Path, chunks: &[(ChunkPos, Tag)]) {
    write_compressed(file, chunks, Compression::Zlib);
}

/// [`write_chunks`] with the chunks' compression, so that a chunk whose
/// payload reaches 256 sectors is stored in its own `c.X.Z.mcc`.
fn write_compressed(file: &Path, chunks: &[(ChunkPos, Tag)], compression: Compression) {
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    let mut region = RegionFile::open_writable(file, true).unwrap();
    for (pos, tag) in chunks {
        let record =
            ChunkRecord::from_nbt(&root(tag.clone()), compression, 1, Limits::default()).unwrap();
        region.write(*pos, &record).unwrap();
    }
}

fn read_chunk(file: &Path, pos: ChunkPos) -> Option<Tag> {
    let mut region = RegionFile::open(file).unwrap();
    region
        .read(pos)
        .unwrap()
        .map(|r| r.nbt(Limits::default()).unwrap().tag)
}

fn entity_chunk(version: i32, x: i32, z: i32) -> Tag {
    Tag::compound([
        ("DataVersion", Tag::Int(version)),
        ("Position", Tag::IntArray(vec![x, z])),
        ("Entities", Tag::List(vec![])),
    ])
}

fn level(world: &TempWorld) -> Vec<String> {
    let _ = world;
    vec![OVERWORLD.into(), NETHER.into()]
}

fn run(world: &TempWorld, erase: bool, recreate: bool) -> Arc<UpgradeProgress> {
    let progress = Arc::new(UpgradeProgress::new());
    work(&world.0, &level(world), &progress, erase, recreate);
    progress
}

fn version_of(file: &Path, pos: ChunkPos) -> i32 {
    as_int(read_chunk(file, pos).unwrap().get("DataVersion"), -1)
}

#[test]
fn old_chunks_are_converted_and_current_ones_skipped() {
    let world = TempWorld::new("convert");
    let file = world.folder(OVERWORLD, "entities").join("r.0.0.mca");
    write_chunks(
        &file,
        &[
            (ChunkPos::new(0, 0), entity_chunk(4000, 0, 0)),
            (ChunkPos::new(0, 1), entity_chunk(CURRENT, 0, 1)),
        ],
    );
    let progress = run(&world, false, false);
    assert!(progress.is_finished());
    assert_eq!(version_of(&file, ChunkPos::new(0, 0)), CURRENT);
    assert_eq!(version_of(&file, ChunkPos::new(0, 1)), CURRENT);
    // The last data type run is the blocks; entities counted 1 converted, 1 skipped.
    assert_eq!(progress.data_fix_type(), Some(UpgradeDataFixType::Chunk));
    assert_eq!(progress.total_chunks(), 0);
}

#[test]
fn counts_of_one_data_type() {
    let world = TempWorld::new("counts");
    let file = world.folder(OVERWORLD, "entities").join("r.0.0.mca");
    write_chunks(
        &file,
        &[
            (ChunkPos::new(0, 0), entity_chunk(4000, 0, 0)),
            (ChunkPos::new(0, 1), entity_chunk(CURRENT, 0, 1)),
        ],
    );
    let progress = Arc::new(UpgradeProgress::new());
    upgrade_levels(
        &world.0,
        &level(&world),
        &progress,
        Builder::new(UpgradeDataFixType::EntityChunk, "entities", false),
        |_, _| {},
    );
    assert_eq!(progress.total_chunks(), 2);
    assert_eq!(progress.converted(), 1);
    assert_eq!(progress.skipped(), 1);
    assert_eq!(progress.status(), UpgradeStatus::Finished);
    assert_eq!(progress.total_progress(), 1.0);
    assert_eq!(progress.dimension_progress(OVERWORLD), 1.0);
    assert_eq!(progress.dimension_progress(NETHER), 0.0);
}

/// `converted = converted && processOnePosition(..)`: after a chunk that was
/// not converted, the rest of its file is left alone.
#[test]
fn a_skipped_chunk_ends_the_files_pass() {
    let world = TempWorld::new("shortcircuit");
    let file = world.folder(OVERWORLD, "entities").join("r.0.0.mca");
    write_chunks(
        &file,
        &[
            (ChunkPos::new(0, 0), entity_chunk(CURRENT, 0, 0)),
            (ChunkPos::new(1, 0), entity_chunk(4000, 1, 0)),
        ],
    );
    let progress = Arc::new(UpgradeProgress::new());
    upgrade_levels(
        &world.0,
        &level(&world),
        &progress,
        Builder::new(UpgradeDataFixType::EntityChunk, "entities", false),
        |_, _| {},
    );
    assert_eq!(progress.total_chunks(), 2);
    assert_eq!(progress.converted(), 0);
    assert_eq!(progress.skipped(), 1);
    assert_eq!(version_of(&file, ChunkPos::new(1, 0)), 4000);
}

#[test]
fn poi_chunks_without_a_version_start_at_1945() {
    let world = TempWorld::new("poi");
    let file = world.folder(NETHER, "poi").join("r.-1.2.mca");
    let pos = ChunkPos::new(-32, 64);
    write_chunks(
        &file,
        &[(
            pos,
            Tag::compound([("Sections", Tag::Compound(Default::default()))]),
        )],
    );
    run(&world, false, false);
    assert_eq!(version_of(&file, pos), CURRENT);
}

#[test]
fn erase_cache_drops_heightmaps_and_light() {
    let world = TempWorld::new("erase");
    let file = world.folder(OVERWORLD, "region").join("r.0.0.mca");
    let pos = ChunkPos::new(0, 0);
    let chunk = |version| {
        Tag::compound([
            ("DataVersion", Tag::Int(version)),
            ("xPos", Tag::Int(0)),
            ("zPos", Tag::Int(0)),
            ("Status", Tag::String("minecraft:full".into())),
            ("isLightOn", Tag::Byte(1)),
            ("Heightmaps", Tag::Compound(Default::default())),
            (
                "sections",
                Tag::List(vec![Tag::compound([
                    ("Y", Tag::Byte(0)),
                    ("SkyLight", Tag::ByteArray(vec![0; 2048])),
                    ("BlockLight", Tag::ByteArray(vec![0; 2048])),
                ])]),
            ),
        ])
    };
    write_chunks(&file, &[(pos, chunk(CURRENT))]);
    // Without the option a current chunk is left as it is.
    let progress = run(&world, false, false);
    assert!(read_chunk(&file, pos).unwrap().get("Heightmaps").is_some());
    assert_eq!(progress.status(), UpgradeStatus::Finished);

    run(&world, true, false);
    let after = read_chunk(&file, pos).unwrap();
    assert!(after.get("Heightmaps").is_none());
    assert!(after.get("isLightOn").is_none());
    let Some(Tag::List(sections)) = after.get("sections") else {
        panic!("sections");
    };
    assert!(sections[0].get("SkyLight").is_none() && sections[0].get("BlockLight").is_none());
    assert!(sections[0].get("Y").is_some());
}

#[test]
fn erase_cache_modifier_reports_changes_only_when_something_went() {
    let pos = ChunkPos::new(3, 4);
    let mut bare = Tag::compound([("xPos", Tag::Int(3)), ("zPos", Tag::Int(4))]);
    assert!(!verify_chunk_pos_and_erase_cache(pos, &mut bare));
    let mut lit = Tag::compound([("xPos", Tag::Int(3)), ("isLightOn", Tag::Byte(1))]);
    assert!(verify_chunk_pos_and_erase_cache(pos, &mut lit));
    assert!(!verify_chunk_pos(pos, &mut lit));
}

#[test]
fn recreating_rewrites_every_file_and_removes_the_new_folder() {
    let world = TempWorld::new("recreate");
    let file = world.folder(OVERWORLD, "entities").join("r.0.0.mca");
    write_chunks(
        &file,
        &[
            (ChunkPos::new(0, 0), entity_chunk(CURRENT, 0, 0)),
            (ChunkPos::new(1, 0), entity_chunk(CURRENT, 1, 0)),
        ],
    );
    let before = fs::read(&file).unwrap();
    let progress = run(&world, false, true);
    assert!(progress.is_finished());
    assert!(!world.folder(OVERWORLD, "new_entities").exists());
    assert_eq!(version_of(&file, ChunkPos::new(1, 0)), CURRENT);
    assert_ne!(
        fs::read(&file).unwrap(),
        before,
        "the file was written anew"
    );
}

#[test]
fn cancel_stops_before_the_next_file() {
    let world = TempWorld::new("cancel");
    let file = world.folder(OVERWORLD, "entities").join("r.0.0.mca");
    write_chunks(&file, &[(ChunkPos::new(0, 0), entity_chunk(4000, 0, 0))]);
    let progress = Arc::new(UpgradeProgress::new());
    let mut upgrader = RegionStorageUpgrader::init(
        Builder::new(UpgradeDataFixType::EntityChunk, "entities", false),
        progress.clone(),
        0,
        OVERWORLD,
        &world.0,
    );
    progress.set_canceled();
    upgrader.upgrade();
    assert_eq!(progress.converted(), 0);
    assert_eq!(version_of(&file, ChunkPos::new(0, 0)), 4000);
}

#[test]
fn the_worker_thread_finishes_and_a_cancel_joins_it() {
    let world = TempWorld::new("thread");
    let file = world.folder(OVERWORLD, "entities").join("r.0.0.mca");
    write_chunks(&file, &[(ChunkPos::new(0, 0), entity_chunk(4000, 0, 0))]);
    let mut upgrader = WorldUpgrader::new(&world.0, world_dimensions(&world.0), false, false);
    while !upgrader.is_finished() {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    upgrader.cancel();
    assert_eq!(version_of(&file, ChunkPos::new(0, 0)), CURRENT);
    assert_eq!(upgrader.levels(), [OVERWORLD, NETHER, END]);
}

#[test]
fn status_keys_follow_vanilla() {
    let progress = UpgradeProgress::new();
    assert_eq!(status_key(&progress), "optimizeWorld.stage.counting");
    progress.reset(UpgradeDataFixType::PoiChunk);
    progress.set_status(UpgradeStatus::Upgrading);
    assert_eq!(status_key(&progress), "optimizeWorld.stage.upgrading.poi");
    progress.set_status(UpgradeStatus::Finished);
    assert_eq!(status_key(&progress), "optimizeWorld.stage.finished.poi");
    progress.set_status(UpgradeStatus::Failed);
    assert_eq!(status_key(&progress), "optimizeWorld.stage.failed");
}

#[test]
fn region_names_and_dimension_folders() {
    assert_eq!(region_coordinates("r.-3.12.mca"), Some((-3, 12)));
    assert_eq!(region_coordinates("r.1.mca"), None);
    assert_eq!(region_coordinates("r.a.1.mca"), None);
    assert_eq!(region_coordinates("r.1.1.mcc"), None);
    let base = Path::new("w");
    assert_eq!(
        dimension_path(base, "mod:sky"),
        Path::new("w/dimensions/mod/sky")
    );
    let world = TempWorld::new("dims");
    fs::create_dir_all(world.0.join("dimensions/mod/sky")).unwrap();
    assert_eq!(
        world_dimensions(&world.0),
        [OVERWORLD, NETHER, END, "mod:sky"]
    );
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// Every file below `dir` by relative path. The region header's timestamp
/// table (bytes 4096..8192) is zeroed: it holds the wall clock of the write.
fn snapshot(dir: &Path, rel: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
    for entry in fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let rel = rel.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            snapshot(&entry.path(), &rel, out);
        } else {
            let mut bytes = fs::read(entry.path()).unwrap();
            if bytes.len() >= 8192 {
                bytes[4096..8192].fill(0);
            }
            out.insert(rel, bytes);
        }
    }
}

/// The folders of [`multi_region_world`]: 10 region files each, 6 chunks a file.
const MULTI_FOLDERS: [(&str, &str); 3] = [
    (OVERWORLD, "entities"),
    (NETHER, "poi"),
    (OVERWORLD, "region"),
];

/// Whether chunk `i` of region file `rx`, `rz` of [`multi_region_world`] is old.
fn is_old(rx: i32, rz: i32, i: i32) -> bool {
    (rx + rz + i) % 3 != 0
}

fn multi_region_world(name: &str) -> TempWorld {
    let world = TempWorld::new(name);
    for (dimension, folder) in MULTI_FOLDERS {
        for rx in -2..3 {
            for rz in 0..2 {
                let file = world
                    .folder(dimension, folder)
                    .join(format!("r.{rx}.{rz}.mca"));
                let chunks: Vec<_> = (0..6)
                    .map(|i| {
                        let (x, z) = (rx * 32 + i, rz * 32 + (i * 3) % 32);
                        let version = if is_old(rx, rz, i) { 4000 } else { CURRENT };
                        let tag = if folder == "region" {
                            Tag::compound([
                                ("DataVersion", Tag::Int(version)),
                                ("xPos", Tag::Int(x)),
                                ("zPos", Tag::Int(z)),
                                ("Status", Tag::String("minecraft:full".into())),
                                ("isLightOn", Tag::Byte(1)),
                                ("Heightmaps", Tag::Compound(Default::default())),
                            ])
                        } else {
                            entity_chunk(version, x, z)
                        };
                        (ChunkPos::new(x, z), tag)
                    })
                    .collect();
                write_chunks(&file, &chunks);
            }
        }
    }
    world
}

/// The counts vanilla's loop leaves for [`multi_region_world`]'s last stage,
/// the `region` folder (each stage starts them again): a file's chunks in
/// order, each converted when old (or when its cache is erased, or always
/// when recreating), and the first one left as it is ends the file's pass.
fn expected_counts(erase: bool, recreate: bool) -> (i32, i32) {
    let (mut converted, mut skipped) = (0, 0);
    for rx in -2..3 {
        for rz in 0..2 {
            for i in 0..6 {
                if is_old(rx, rz, i) || recreate || erase {
                    converted += 1;
                } else {
                    skipped += 1;
                    break;
                }
            }
        }
    }
    (converted, skipped)
}

/// The output of a many-threaded run equals the one-thread run byte for byte,
/// and both give vanilla's counts and end at full progress.
#[test]
fn parallel_output_is_byte_identical_to_one_thread() {
    // Each run mutates only its private copy, so all modes can copy the same source.
    let source = multi_region_world("par-src");
    for (erase, recreate) in [(false, false), (true, false), (false, true)] {
        let mut results = Vec::new();
        for threads in [1, 4] {
            let copy = TempWorld::new(&format!("par-{threads}"));
            copy_tree(&source.0, &copy.0);
            let progress = Arc::new(UpgradeProgress::new());
            work_with_threads(&copy.0, &level(&copy), &progress, erase, recreate, threads);
            assert!(progress.is_finished());
            assert_eq!(
                (progress.converted(), progress.skipped()),
                expected_counts(erase, recreate),
                "threads={threads} erase={erase} recreate={recreate}"
            );
            assert_eq!(progress.total_progress(), 1.0);
            assert_eq!(progress.dimension_progress(NETHER), 1.0);
            let mut files = BTreeMap::new();
            snapshot(&copy.0, Path::new(""), &mut files);
            results.push(files);
        }
        assert_eq!(results[0].len(), 30, "erase={erase} recreate={recreate}");
        assert!(
            results[0] == results[1],
            "erase={erase} recreate={recreate}: the files differ"
        );
    }
}

/// A cancel during a file stops every worker before its next chunk, not at
/// the end of its file; a canceled `recreate_region_files` run leaves the
/// old files as they were.
#[test]
fn cancel_stops_every_worker_within_a_chunk() {
    for recreate in [false, true] {
        let world = TempWorld::new(&format!("cancel-workers-{recreate}"));
        for rx in 0..8 {
            let file = world
                .folder(OVERWORLD, "entities")
                .join(format!("r.{rx}.0.mca"));
            let chunks: Vec<_> = (0..32)
                .map(|x| {
                    let pos = ChunkPos::new(rx * 32 + x, 0);
                    (pos, entity_chunk(4000, pos.x, pos.z))
                })
                .collect();
            write_chunks(&file, &chunks);
        }
        let mut before = BTreeMap::new();
        snapshot(&world.0, Path::new(""), &mut before);
        let progress = Arc::new(UpgradeProgress::new());
        let mut builder = Builder::new(UpgradeDataFixType::EntityChunk, "entities", recreate);
        builder.threads = 4;
        let canceling = progress.clone();
        // The first chunk upgraded cancels the run.
        builder.add_tag_modifier(
            CURRENT,
            Box::new(move |_, _| {
                canceling.set_canceled();
                false
            }),
        );
        let mut upgrader =
            RegionStorageUpgrader::init(builder, progress.clone(), 0, OVERWORLD, &world.0);
        upgrader.upgrade();
        let looked_at = progress.converted() + progress.skipped();
        // At most the chunk each of the 4 workers had in hand; a file is 32.
        assert!(
            (1..=4).contains(&looked_at),
            "recreate={recreate}: {looked_at} chunks after the cancel"
        );
        if recreate {
            let mut after = BTreeMap::new();
            snapshot(&world.0, Path::new(""), &mut after);
            assert!(before == after, "a canceled recreate changed the world");
        }
    }
}

/// Pseudo-random bytes: zlib cannot shrink them, so a chunk carrying them is
/// 256 sectors or more whatever the compression and lives in its own `.mcc`.
fn incompressible(len: usize) -> Vec<u8> {
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    (0..len)
        .map(|_| {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (state >> 33) as u8
        })
        .collect()
}

/// An entity chunk whose payload is 256 sectors or more: the region keeps it
/// in `c.X.Z.mcc`, not inline.
fn oversized_entity_chunk(version: i32, noise: &[u8]) -> Tag {
    Tag::compound([
        ("DataVersion", Tag::Int(version)),
        ("Position", Tag::IntArray(vec![0, 0])),
        ("Entities", Tag::List(vec![])),
        ("Noise", Tag::ByteArray(noise.to_vec())),
    ])
}

/// A chunk stored in its own `.mcc` keeps its payload through the upgrade,
/// its compression with it: a stop between the payload and the new stub
/// leaves the old stub reading a stream it can decode.
#[test]
fn an_oversized_chunk_survives_the_upgrade_with_its_compression() {
    let noise = incompressible(1_100_000);
    for recreate in [false, true] {
        let world = TempWorld::new(&format!("mcc-{recreate}"));
        let folder = world.folder(OVERWORLD, "entities");
        let file = folder.join("r.0.0.mca");
        let pos = ChunkPos::new(0, 0);
        write_compressed(
            &file,
            &[(pos, oversized_entity_chunk(4000, &noise))],
            Compression::None,
        );
        assert!(
            folder.join("c.0.0.mcc").is_file(),
            "recreate={recreate}: the chunk was not stored outside the region"
        );

        let progress = run(&world, false, recreate);
        assert!(progress.is_finished(), "recreate={recreate}");

        let after = read_chunk(&file, pos);
        let after = after.unwrap_or_else(|| panic!("recreate={recreate}: the chunk was lost"));
        assert_eq!(
            as_int(after.get("DataVersion"), -1),
            CURRENT,
            "recreate={recreate}"
        );
        assert_eq!(
            after.get("Noise"),
            Some(&Tag::ByteArray(noise.clone())),
            "recreate={recreate}: the payload changed"
        );
        assert!(!world.folder(OVERWORLD, "new_entities").exists());
        assert!(
            folder.join("c.0.0.mcc").is_file(),
            "recreate={recreate}: the payload is not beside the region file"
        );
        let record = RegionFile::open(&file).unwrap().read(pos).unwrap();
        let record = record.unwrap_or_else(|| panic!("recreate={recreate}: no record"));
        assert_eq!(
            record.compression,
            Compression::None,
            "recreate={recreate}: the stub names a compression the payload is not in"
        );
    }
}

/// A `new_` file left by a run that stopped before publishing is not the
/// file this run writes: its header named a chunk the world no longer has.
#[test]
fn a_leftover_new_file_publishes_nothing_it_does_not_hold() {
    let world = TempWorld::new("leftover");
    let (kept, removed) = (ChunkPos::new(0, 0), ChunkPos::new(1, 0));
    let file = world.folder(OVERWORLD, "entities").join("r.0.0.mca");
    write_chunks(&file, &[(kept, entity_chunk(4000, 0, 0))]);
    let leftover = world.folder(OVERWORLD, "new_entities").join("r.0.0.mca");
    write_chunks(&leftover, &[(removed, entity_chunk(CURRENT, 1, 0))]);

    let progress = run(&world, false, true);

    assert!(progress.is_finished());
    assert!(
        read_chunk(&file, removed).is_none(),
        "the leftover published a chunk the world does not have"
    );
    assert_eq!(version_of(&file, kept), CURRENT);
}

/// A leftover that will not go stops its file. Windows only: a handle held
/// without `FILE_SHARE_DELETE` makes the remove fail with a sharing violation
/// while the write-open still gets in, which is the one way here to fail the
/// remove and keep the open working. Writing into a leftover it could not
/// remove keeps the leftover's header, whose chunk the world no longer has.
#[cfg(windows)]
#[test]
fn a_leftover_that_will_not_go_stops_the_file() {
    use std::os::windows::fs::OpenOptionsExt;

    let world = TempWorld::new("leftover-held");
    let (kept, stale) = (ChunkPos::new(0, 0), ChunkPos::new(1, 0));
    let file = world.folder(OVERWORLD, "entities").join("r.0.0.mca");
    write_chunks(&file, &[(kept, entity_chunk(4000, 0, 0))]);
    let leftover = world.folder(OVERWORLD, "new_entities").join("r.0.0.mca");
    write_chunks(&leftover, &[(stale, entity_chunk(CURRENT, 1, 0))]);
    // `FILE_SHARE_READ | FILE_SHARE_WRITE`, no `FILE_SHARE_DELETE`.
    let held = fs::OpenOptions::new()
        .read(true)
        .share_mode(3)
        .open(&leftover)
        .unwrap();

    let progress = run(&world, false, true);

    assert!(progress.is_finished());
    assert!(
        read_chunk(&leftover, kept).is_none(),
        "the run went on into a leftover it could not remove"
    );
    assert!(
        read_chunk(&file, stale).is_none(),
        "the leftover published a chunk the world does not have"
    );
    assert_eq!(version_of(&file, kept), 4000);
    drop(held);
}

/// Links `link` to the directory `target`: a symlink where the user may make
/// one, else a junction (Windows), which needs no privilege. Fails when the
/// machine allows neither; the caller then has no aliased path to test with.
fn link_dir(link: &Path, target: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)
    }
    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_dir(target, link)
            .or_else(|_| horizon_threads::junction(target, link))
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (link, target);
        Err(io::Error::other("no way to link a directory"))
    }
}

/// A staging folder that links to the live one is detached before the run,
/// so the cancel after the first chunk finds the live region untouched:
/// written through the link, the leftover removal would have deleted the
/// live file and the rewrite would have left only the first chunk in it.
#[test]
fn an_aliased_staging_folder_keeps_the_live_region() {
    let world = TempWorld::new("aliased-staging");
    let live = world.folder(OVERWORLD, "entities");
    let file = live.join("r.0.0.mca");
    let positions: Vec<ChunkPos> = (0..3).map(|x| ChunkPos::new(x, 0)).collect();
    let chunks: Vec<(ChunkPos, Tag)> = positions
        .iter()
        .map(|pos| (*pos, entity_chunk(4000, pos.x, pos.z)))
        .collect();
    write_chunks(&file, &chunks);
    let staging = resolve_recreate_directory(&live);
    if let Err(e) = link_dir(&staging, &live) {
        eprintln!("an_aliased_staging_folder_keeps_the_live_region: cannot create the link: {e}");
        return;
    }

    let progress = Arc::new(UpgradeProgress::new());
    let mut builder = Builder::new(UpgradeDataFixType::EntityChunk, "entities", true);
    builder.threads = 1;
    let canceling = progress.clone();
    // The first chunk upgraded cancels the run.
    builder.add_tag_modifier(
        CURRENT,
        Box::new(move |_, _| {
            canceling.set_canceled();
            false
        }),
    );
    let mut upgrader =
        RegionStorageUpgrader::init(builder, progress.clone(), 0, OVERWORLD, &world.0);
    upgrader.upgrade();

    for pos in &positions {
        assert_eq!(
            version_of(&file, *pos),
            4000,
            "the live region lost chunk {pos:?} through the aliased staging folder"
        );
    }
    assert!(!staging.exists(), "the aliased staging path stayed");
}

/// A live folder that is itself a link makes its staging folder — a real
/// one holding the region files — canonicalize to it: it is not a link and
/// not empty, so neither `remove_dir` nor `remove_file` takes it and the
/// folder stays out of the run. Closing the run must not remove it either.
#[test]
fn a_staging_folder_that_is_the_live_one_is_left_alone() {
    let world = TempWorld::new("staging-is-live");
    let live = world.folder(OVERWORLD, "entities");
    let staging = resolve_recreate_directory(&live);
    let positions: Vec<ChunkPos> = (0..3).map(|x| ChunkPos::new(x, 0)).collect();
    let chunks: Vec<(ChunkPos, Tag)> = positions
        .iter()
        .map(|pos| (*pos, entity_chunk(4000, pos.x, pos.z)))
        .collect();
    write_chunks(&staging.join("r.0.0.mca"), &chunks);
    if let Err(e) = link_dir(&live, &staging) {
        eprintln!(
            "a_staging_folder_that_is_the_live_one_is_left_alone: cannot create the link: {e}"
        );
        return;
    }

    let progress = run(&world, false, true);

    assert!(progress.is_finished());
    for pos in &positions {
        assert_eq!(
            version_of(&live.join("r.0.0.mca"), *pos),
            4000,
            "the run rewrote chunk {pos:?} of a folder it had to leave alone"
        );
    }
    assert!(staging.exists(), "closing removed the real staging folder");
}
