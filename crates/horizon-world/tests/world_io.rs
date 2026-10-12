use horizon_core::position::{BlockPos, ChunkPos};
use horizon_world::{
    DATA_VERSION,
    chunk::{BlockState, ChunkData, PalettedContainer},
    compression::Compression,
    nbt::{JavaString, Limits, NamedTag, Tag},
    region::{ChunkRecord, RegionFile, RegionWriter},
};
use std::{
    fs,
    io::{Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "horizon-world-test-{}-{stamp}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn region(&self) -> PathBuf {
        self.0.join("r.-1.0.mca")
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn record(len: usize) -> ChunkRecord {
    ChunkRecord {
        compression: Compression::None,
        compressed: vec![37; len],
        timestamp: 0x8000_0001,
    }
}

#[test]
fn external_threshold_negative_coordinates_and_timestamps_roundtrip() {
    let dir = Directory::new();
    let a = ChunkPos::new(-32, 0);
    let b = ChunkPos::new(-1, 31);
    let inline = record(255 * 4096 - 5);
    let external = record(255 * 4096 - 4);
    let mut writer = RegionWriter::create(dir.region()).unwrap();
    writer.write(a, &inline).unwrap();
    writer.write(b, &external).unwrap();
    writer.finish().unwrap();
    assert!(!dir.0.join("c.-32.0.mcc").exists());
    assert!(dir.0.join("c.-1.31.mcc").exists());
    let mut reader = RegionFile::open(dir.region()).unwrap();
    assert_eq!(
        reader.chunks().collect::<Vec<_>>(),
        [(a, inline.timestamp), (b, external.timestamp)]
    );
    for (pos, expected) in [(a, inline), (b, external)] {
        let actual = reader.read(pos).unwrap().unwrap();
        assert_eq!(actual.compressed, expected.compressed);
        assert_eq!(actual.timestamp, expected.timestamp);
    }
    assert!(reader.read(ChunkPos::new(-31, 0)).unwrap().is_none());
    assert!(reader.read(ChunkPos::new(0, 0)).is_err());
}

#[test]
fn failed_writer_cannot_commit_and_removes_its_own_external_files() {
    let dir = Directory::new();
    let mut writer = RegionWriter::create(dir.region()).unwrap();
    writer
        .write(ChunkPos::new(-1, 0), &record(1_100_000))
        .unwrap();
    assert!(dir.0.join("c.-1.0.mcc").exists());
    assert!(writer.write(ChunkPos::new(0, 0), &record(4)).is_err());
    assert!(writer.finish().is_err());
    assert!(!dir.region().exists());
    assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 0);
}

#[test]
fn existing_external_and_concurrent_destination_are_preserved() {
    let dir = Directory::new();
    let external = dir.0.join("c.-1.0.mcc");
    fs::write(&external, b"owned by someone else").unwrap();
    let mut writer = RegionWriter::create(dir.region()).unwrap();
    assert!(
        writer
            .write(ChunkPos::new(-1, 0), &record(1_100_000))
            .is_err()
    );
    drop(writer);
    assert_eq!(fs::read(&external).unwrap(), b"owned by someone else");
    let mut writer = RegionWriter::create(dir.region()).unwrap();
    writer.write(ChunkPos::new(-32, 0), &record(3)).unwrap();
    fs::write(dir.region(), b"another writer won").unwrap();
    assert!(writer.finish().is_err());
    assert_eq!(fs::read(dir.region()).unwrap(), b"another writer won");
    assert!(RegionWriter::create(dir.region()).is_err());
}

fn malformed_region(path: &Path, first: u32, second: u32, payload_len: u32, codec: u8) {
    let mut file = fs::File::create(path).unwrap();
    file.set_len(12288).unwrap();
    file.write_all(&first.to_be_bytes()).unwrap();
    file.write_all(&second.to_be_bytes()).unwrap();
    file.seek(SeekFrom::Start(8192)).unwrap();
    file.write_all(&payload_len.to_be_bytes()).unwrap();
    file.write_all(&[codec]).unwrap();
}

#[test]
fn corrupt_allocations_are_dropped_and_bad_payloads_fail() {
    let dir = Directory::new();
    // RegionFile's constructor drops (and logs) entries whose sector overlaps the
    // header, that have no sectors or that start past the end of the file; it
    // does not check overlaps between chunks.
    for (first, second, chunks) in [(257, 0, 0), (512, 0, 0), (1025, 0, 0), (513, 513, 2)] {
        malformed_region(&dir.region(), first, second, 1, 3);
        let reader = RegionFile::open(dir.region()).unwrap();
        assert_eq!(reader.chunks().count(), chunks);
    }
    // An empty or truncated file reads as a region without chunks.
    fs::write(dir.region(), b"").unwrap();
    assert_eq!(RegionFile::open(dir.region()).unwrap().chunks().count(), 0);
    fs::write(dir.region(), [0, 0, 2, 1]).unwrap();
    assert_eq!(RegionFile::open(dir.region()).unwrap().chunks().count(), 0);
    for (len, codec) in [
        (0, 3),
        (4093, 3),
        (u32::MAX, 3),
        (1, 127),
        (2, 131),
        (1, 131),
    ] {
        malformed_region(&dir.region(), 513, 0, len, codec);
        let mut reader = RegionFile::open(dir.region()).unwrap();
        assert!(reader.read(ChunkPos::new(-32, 0)).is_err());
    }
}

#[test]
fn chunk_storage_edit_preserves_unknown_fields_and_other_blocks() {
    let palette =
        PalettedContainer::uniform(Tag::String(JavaString::from("minecraft:air")), 4096, 4)
            .unwrap();
    let root = NamedTag {
        name: "".into(),
        tag: Tag::compound([
            ("DataVersion", Tag::Int(DATA_VERSION)),
            ("xPos", Tag::Int(-1)),
            ("zPos", Tag::Int(-1)),
            ("extra", Tag::ByteArray(vec![1, 2, 3])),
            (
                "sections",
                Tag::List(vec![Tag::compound([
                    ("Y", Tag::Byte(-4)),
                    ("block_states", palette.to_nbt()),
                    ("unknown", Tag::Long(99)),
                ])]),
            ),
        ]),
    };
    let mut chunk = ChunkData::new(root).unwrap();
    let p = BlockPos::new(-1, -64, -1);
    let state = BlockState {
        name: "minecraft:oak_log".to_owned(),
        properties: [("axis".to_owned(), "x".to_owned())].into(),
    };
    assert_eq!(
        chunk.set_block_data(p, state.clone()).unwrap().name,
        "minecraft:air"
    );
    assert_eq!(chunk.block(p).unwrap(), Some(state));
    assert_eq!(
        chunk.block(p.offset(-1, 0, 0)).unwrap().unwrap().name,
        "minecraft:air"
    );
    assert!(
        chunk
            .set_block_data(
                p.offset(1, 0, 0),
                BlockState {
                    name: "minecraft:stone".into(),
                    properties: Default::default()
                }
            )
            .is_err()
    );
    let root = chunk.into_nbt();
    assert_eq!(root.tag.get("extra"), Some(&Tag::ByteArray(vec![1, 2, 3])));
    let Some(Tag::List(sections)) = root.tag.get("sections") else {
        panic!()
    };
    assert_eq!(sections[0].get("unknown"), Some(&Tag::Long(99)));
    let record = ChunkRecord::from_nbt(&root, Compression::Lz4, 17, Limits::default()).unwrap();
    assert_eq!(record.nbt(Limits::default()).unwrap(), root);
}
