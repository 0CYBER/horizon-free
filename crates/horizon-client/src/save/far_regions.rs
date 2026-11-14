//! Saved chunks for far readers: far terrain's sampling threads and a
//! player's far ring (`lan::far`).
//!
//! A far read stays out of the gameplay region cache: the far view spans
//! more region files than the cache keeps open, and would push the ones the
//! game plays in out of it. It opened the region file for each chunk
//! instead, which reads the file's 8 KiB header for a record about as long
//! (#6386: tens of gigabytes read under a far view). The header is kept
//! here, one a region, and a far read opens the file for the record alone.
//!
//! A kept header is only good until the region is written: a write moves the
//! chunk and frees its old sectors, which the next chunk written can take
//! (`RegionFile::write`). So each region has a write stamp, in the cache the
//! writer goes through (`writing`), and a far read uses a kept header only
//! where the stamp says that nothing was written since it was read.
use super::*;
use horizon_world::region::RegionHeader;
use std::time::{Duration, Instant};

/// How long a far read trusts a kept header that this process did not write
/// under: what another program saved shows in the far view this long after.
const HEADER_TTL: Duration = Duration::from_secs(10);
/// The kept headers (8 KiB each) past which the ones too old to be used
/// again are dropped.
const KEPT_HEADERS: usize = 256;

/// A region file's header, `None` without the file, as of `checked` and read
/// while the region's write stamp was `written`.
struct FarRegion {
    checked: Instant,
    written: u64,
    header: Option<Arc<RegionHeader>>,
}

#[derive(Default)]
pub(super) struct FarRegions(Mutex<crate::fast_hash::FxHashMap<(i32, i32), FarRegion>>);

/// The writer at work on a region, from `writing` until it is dropped.
pub(super) struct Writing<'a>(&'a RegionCache, (i32, i32));

/// Count a step of the writer on a region: once before it writes the region's
/// chunks and once when it is done with them, also when a write failed. The
/// stamp is odd while they are being written, and never comes back to a value
/// it had: a header read under one stamp is right for as long as the stamp
/// stays.
fn mark_written(cache: &RegionCache, region: (i32, i32)) {
    if let Ok(mut cache) = cache.lock() {
        *cache.written.entry(region).or_default() += 1;
    }
}
pub(super) fn writing(cache: &RegionCache, region: (i32, i32)) -> Writing<'_> {
    mark_written(cache, region);
    Writing(cache, region)
}
impl Drop for Writing<'_> {
    fn drop(&mut self) {
        mark_written(self.0, self.1);
    }
}

impl FarRegions {
    /// The chunk's record at the place the region's kept header gives it. The
    /// header is read from `path` where none is kept, or the kept one is
    /// older than `HEADER_TTL` or was read under another stamp than `written`.
    fn read(
        &self,
        chunk: ChunkPos,
        written: u64,
        path: impl FnOnce() -> PathBuf,
    ) -> Option<ChunkRecord> {
        let region = (chunk.x >> 5, chunk.z >> 5);
        let header = {
            let mut regions = self.0.lock().unwrap();
            match regions.get(&region) {
                Some(kept) if kept.written == written && kept.checked.elapsed() < HEADER_TTL => {
                    kept.header.clone()
                }
                _ => {
                    if regions.len() >= KEPT_HEADERS {
                        regions.retain(|_, kept| kept.checked.elapsed() < HEADER_TTL);
                    }
                    let header = RegionHeader::read(path()).ok().map(Arc::new);
                    let kept = FarRegion {
                        checked: Instant::now(),
                        written,
                        header: header.clone(),
                    };
                    regions.insert(region, kept);
                    header
                }
            }
        };
        header?.read_chunk(chunk).ok()?
    }
}

impl SaveDir {
    /// The saved chunk `x z` for a far reader, brought to the current
    /// version: its queued write, else its record in the region file. `None`
    /// for a chunk not saved, without opening a file where there is none.
    pub(super) fn far_record(&self, x: i32, z: i32) -> Option<NamedTag> {
        let pending = self
            .pending
            .lock()
            .ok()?
            .get(&(Folder::Region, x, z))
            .map(|(_, record)| record.clone());
        if let Some(record) = pending {
            return record.tag().map(|tag| (*tag).clone());
        }
        let mut root = self.far_region_record(x, z)?;
        self.upgrade_record(Folder::Region, &mut root).ok()?;
        Some(root)
    }

    fn far_region_record(&self, x: i32, z: i32) -> Option<NamedTag> {
        let region = (x >> 5, z >> 5);
        let stamp = || {
            let cache = self.regions.lock().ok()?;
            Some(cache.written.get(&region).copied().unwrap_or(0))
        };
        let before = stamp()?;
        if before & 1 == 0 {
            let path = || self.region_path("region", region.0, region.1);
            let record = self.far_regions.read(ChunkPos::new(x, z), before, path);
            // The same stamp after the read: no write began since the header
            // was read, so the chunk was where the header says all along.
            if stamp()? == before {
                return record?.nbt(Limits::default()).ok();
            }
        }
        // The region is being written, or was a moment ago. The writer's own
        // open file is in the gameplay cache, and exact under its lock.
        read_region_record(&self.root, &self.regions, "region", x, z).ok()?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn save_dir(name: &str) -> (PathBuf, SaveDir) {
        let dir = std::env::temp_dir().join(format!("horizon-far-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        (dir.clone(), SaveDir::new(dir, LevelShape::OVERWORLD))
    }
    /// A current record that says which chunk it is and which of its saves.
    fn record((x, z): (i32, i32), version: i32) -> Record {
        Record::Tag(Arc::new(NamedTag {
            name: "".into(),
            tag: Tag::compound([
                ("DataVersion", Tag::Int(DATA_VERSION)),
                ("xPos", Tag::Int(x)),
                ("zPos", Tag::Int(z)),
                ("version", Tag::Int(version)),
            ]),
        }))
    }
    fn write(save: &SaveDir, records: &[((i32, i32), &Record)]) {
        write_region_records(&save.root, &save.regions, "region", records).unwrap();
    }
    /// What a far read finds for the chunk: `(xPos, zPos, version)`.
    fn far(save: &SaveDir, (x, z): (i32, i32)) -> Option<(i64, i64, i64)> {
        let root = save.far_record(x, z)?;
        let field = |name| number(root.tag.get(name)).unwrap();
        Some((field("xPos"), field("zPos"), field("version")))
    }

    /// The writer puts a chunk in the first free sectors and then frees its
    /// old ones: the header kept from before names a place that now holds
    /// another chunk.
    #[test]
    fn a_far_read_finds_a_chunk_where_the_writer_moved_it() {
        let (dir, save) = save_dir("moved");
        let (a, b, c) = ((3, 5), (4, 5), (5, 5));
        assert_eq!(far(&save, a), None);
        // The region's first chunk: the sector after the header.
        write(&save, &[(a, &record(a, 1))]);
        assert_eq!(far(&save, a), Some((3, 5, 1)), "saved since the last read");
        assert_eq!(far(&save, b), None);
        // `a` moves to the next free sector, and `b` takes the one it left.
        write(&save, &[(a, &record(a, 2)), (b, &record(b, 1))]);
        assert_eq!(far(&save, a), Some((3, 5, 2)));
        assert_eq!(far(&save, b), Some((4, 5, 1)));
        // `a` is gone, and `c` takes its sector.
        write(&save, &[(a, &Record::Delete), (c, &record(c, 1))]);
        assert_eq!(far(&save, a), None);
        assert_eq!(far(&save, c), Some((5, 5, 1)));
        assert_eq!(far(&save, b), Some((4, 5, 1)));
        let _ = fs::remove_dir_all(&dir);
    }

    /// #6386: each far read opened the region file, and so read its 8 KiB
    /// header again, for a record that is about as long.
    #[cfg(target_os = "linux")]
    #[test]
    fn far_reads_of_a_region_read_its_header_once() {
        // The bytes this thread asked the system for (`man 5 proc_pid_io`).
        let read_so_far = || -> Option<u64> {
            let io = fs::read_to_string("/proc/thread-self/io").ok()?;
            let line = io.lines().find_map(|line| line.strip_prefix("rchar:"))?;
            line.trim().parse().ok()
        };
        if read_so_far().is_none() {
            return;
        }
        let (dir, save) = save_dir("bytes");
        let chunks: Vec<(i32, i32)> = (0..64).map(|i| (-32 + i % 8, 64 + i / 8)).collect();
        let records: Vec<Record> = chunks.iter().map(|&at| record(at, 1)).collect();
        let batch: Vec<_> = chunks.iter().copied().zip(&records).collect();
        write(&save, &batch);
        let file = fs::metadata(save.region_path("region", -1, 2))
            .unwrap()
            .len();
        assert!(
            file >= 8192 + 64 * 4096,
            "a sector a chunk after the header"
        );

        let before = read_so_far().unwrap();
        for &(x, z) in &chunks {
            assert_eq!(far(&save, (x, z)), Some((x.into(), z.into(), 1)));
        }
        let read = read_so_far().unwrap() - before;
        // The header and 64 records of under a sector each. A header a chunk
        // is 64 * 8 KiB before the first record: twice the file.
        assert!(
            read <= file,
            "64 far reads took {read} bytes from a region file of {file}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// Two chunks that the writer keeps moving into each other's old sectors,
    /// read meanwhile: a far read may find either save of a chunk, never the
    /// other chunk.
    #[test]
    fn far_reads_meet_no_other_chunk_while_the_writer_moves_them() {
        let (dir, save) = save_dir("moving");
        let save = Arc::new(save);
        let (a, b) = ((-7, 40), (-6, 40));
        write(&save, &[(a, &record(a, 0)), (b, &record(b, 0))]);
        let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let readers: Vec<_> = (0..3)
            .map(|_| {
                let (save, done) = (save.clone(), done.clone());
                std::thread::spawn(move || {
                    let mut reads = 0u32;
                    while !done.load(Ordering::Relaxed) {
                        for at in [a, b] {
                            let (x, z, _) = far(&save, at).expect("a saved chunk");
                            assert_eq!((x, z), (at.0.into(), at.1.into()));
                            reads += 1;
                        }
                    }
                    reads
                })
            })
            .collect();
        for version in 1..=200 {
            write(&save, &[(a, &record(a, version)), (b, &record(b, version))]);
        }
        done.store(true, Ordering::Relaxed);
        for reader in readers {
            assert!(reader.join().expect("a far read met another chunk") > 0);
        }
        assert_eq!(far(&save, a), Some((-7, 40, 200)));
        assert_eq!(far(&save, b), Some((-6, 40, 200)));
        let _ = fs::remove_dir_all(&dir);
    }
}
