//! Simulated power cuts against `RegionFile`, from a fixed seed, with no root,
//! VM or disk image:
//!
//!     cargo run --profile fast -p horizon-world --example region_power_cut -- \
//!         [--seed 1] [--cuts 2000] [--chunks 96] [--batch 24] [--rounds 12] [--sync-folder]
//!
//! The workload replays `SaveDir`'s writer (`write_region_records`): each round
//! writes a batch of chunks with `RegionFile::write`, then `sync`s each region
//! once, and its chunks count as acknowledged. Records name their chunk and a
//! version and change size, so chunks move between sectors; a second region
//! starts halfway through. After every call the harness reads the files back
//! and keeps the 512-byte sectors that changed, so a cut can fall after any
//! call, even between a chunk's sectors and the header pointing to them.
//!
//! A cut keeps what the last sync made durable; every unit changed since then
//! independently takes any of its versions since that sync (the synced one
//! included), as writeback and the disk may reorder writes. Models:
//!
//! - `ordered`: every call reached the disk in order (a killed process): the
//!   control, which must lose nothing;
//! - `pages`: 4 KiB pages, each kept or not whole (reordering only);
//! - `torn`: 512-byte sectors, so a page can also be cut short;
//! - `dir`: `torn`, and a file made since its folder was last synced may be
//!   gone (neither `RegionFile` nor `SaveDir` syncs the folder).
//!
//! Then `RegionFile::open` reads every acknowledged chunk back. Its last
//! acknowledged version or a later one, with the payload that version was
//! written with, is fine; nothing, an older version, an unreadable record or a
//! payload with damaged bytes is lost; another chunk's record is a wrong-key
//! read. Another store drives the same [`Recorder`] (`changed` after each
//! write, `synced`, `folder_synced`, `acknowledged`).
use horizon_core::{
    nbt::{Limits, NamedTag, Tag},
    position::ChunkPos,
};
use horizon_world::{
    compression::Compression,
    region::{ChunkRecord, RegionFile},
};
use std::{
    collections::{BTreeMap, HashMap, btree_map::Entry},
    fs, io,
    path::{Path, PathBuf},
};

const SECTOR: usize = 512;

/// A folder this run made in the temp folder, removed with everything in it
/// when dropped. A name already taken is skipped, never cleared.
struct Scratch(PathBuf);
impl Scratch {
    fn new(name: &str) -> io::Result<Self> {
        let mut n = 0u32;
        loop {
            let dir = std::env::temp_dir().join(format!("{name}-{}-{n}", std::process::id()));
            match fs::create_dir(&dir) {
                Ok(()) => return Ok(Self(dir)),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => n += 1,
                Err(e) => return Err(e),
            }
        }
    }
}
impl std::ops::Deref for Scratch {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.0
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// splitmix64: a seeded, portable random stream for the workload and the cuts.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

enum Event {
    /// A file's 512-byte sectors that one call changed, and its new length.
    Changed {
        file: usize,
        sectors: Vec<(usize, Vec<u8>)>,
        len: usize,
    },
    Synced(usize),
    /// The folder synced: the files made so far keep their names.
    FolderSynced,
    /// The chunks (and versions) the caller was told are on the disk.
    Acknowledged(Vec<((i32, i32), u32)>),
}

/// The calls of a workload, recorded against the real files it wrote.
#[derive(Default)]
struct Recorder {
    files: Vec<(PathBuf, Vec<u8>)>,
    events: Vec<Event>,
    /// Each chunk version's payload as written: nothing in a record checks
    /// its bytes, so a torn sector inside it would otherwise read as fine.
    payloads: HashMap<((i32, i32), u32), Vec<u8>>,
}
impl Recorder {
    /// Records what changed in `path` since it was last seen.
    fn changed(&mut self, path: &Path) -> io::Result<()> {
        let now = fs::read(path)?;
        if !self.files.iter().any(|(p, _)| p == path) {
            self.files.push((path.to_owned(), Vec::new()));
        }
        let file = self.files.iter().position(|(p, _)| p == path).unwrap();
        let before = &self.files[file].1;
        let sectors = (0..now.len().div_ceil(SECTOR))
            .filter_map(|s| {
                let range = s * SECTOR..((s + 1) * SECTOR).min(now.len());
                (before.get(range.clone()) != Some(&now[range.clone()]))
                    .then(|| (s, now[range].to_vec()))
            })
            .collect();
        self.events.push(Event::Changed {
            file,
            sectors,
            len: now.len(),
        });
        self.files[file].1 = now;
        Ok(())
    }
    fn synced(&mut self, path: &Path) {
        if let Some(i) = self.files.iter().position(|(p, _)| p == path) {
            self.events.push(Event::Synced(i));
        }
    }
    fn folder_synced(&mut self) {
        self.events.push(Event::FolderSynced);
    }
    fn acknowledged(&mut self, chunks: Vec<((i32, i32), u32)>) {
        self.events.push(Event::Acknowledged(chunks));
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Model {
    Ordered,
    Pages,
    Torn,
    Dir,
}

#[derive(Default)]
struct FileState {
    current: Vec<u8>,
    durable: Vec<u8>,
    /// Each unit's versions since the last sync, the durable one first.
    versions: BTreeMap<usize, Vec<Vec<u8>>>,
    /// Its folder was synced after it was made.
    named: bool,
}

#[derive(Default, Debug, PartialEq)]
struct Outcome {
    lost: usize,
    wrong_key: usize,
}

/// The files a cut after the first `cut` events could leave, and the chunks
/// acknowledged by then.
fn crash(
    recorder: &Recorder,
    cut: usize,
    model: Model,
    rng: &mut Rng,
) -> (Vec<Option<Vec<u8>>>, HashMap<(i32, i32), u32>) {
    let unit = if model == Model::Pages { 4096 } else { SECTOR };
    let mut files: Vec<FileState> = recorder
        .files
        .iter()
        .map(|_| FileState::default())
        .collect();
    let mut acknowledged = HashMap::new();
    for event in &recorder.events[..cut] {
        match event {
            Event::Changed { file, sectors, len } => {
                let state = &mut files[*file];
                state.current.resize(*len, 0);
                let mut touched = Vec::new();
                for (s, bytes) in sectors {
                    state.current[s * SECTOR..s * SECTOR + bytes.len()].copy_from_slice(bytes);
                    touched.push(s * SECTOR / unit);
                }
                touched.dedup();
                for u in touched {
                    let at = |image: &[u8]| {
                        let mut page = vec![0; unit];
                        let end = ((u + 1) * unit).min(image.len());
                        if u * unit < end {
                            page[..end - u * unit].copy_from_slice(&image[u * unit..end]);
                        }
                        page
                    };
                    let durable = at(&state.durable);
                    let new = at(&state.current);
                    state
                        .versions
                        .entry(u)
                        .or_insert_with(|| vec![durable])
                        .push(new);
                }
            }
            Event::Synced(file) => {
                let state = &mut files[*file];
                state.durable = state.current.clone();
                state.versions.clear();
            }
            Event::FolderSynced => files.iter_mut().for_each(|f| f.named = true),
            Event::Acknowledged(chunks) => acknowledged.extend(chunks.iter().copied()),
        }
    }
    let images = files
        .into_iter()
        .map(|state| {
            // Not made yet, or under `dir` made and its name not yet durable.
            if state.current.is_empty()
                || (model == Model::Dir && !state.named && rng.below(2) == 0)
            {
                return None;
            }
            let len = if rng.below(2) == 0 {
                state.durable.len()
            } else {
                state.current.len()
            };
            if model == Model::Ordered {
                return Some(state.current);
            }
            let mut image = state.durable.clone();
            image.resize(state.current.len(), 0);
            for (u, versions) in &state.versions {
                let version = &versions[rng.below(versions.len())];
                let end = ((u + 1) * unit).min(image.len());
                image[u * unit..end].copy_from_slice(&version[..end - u * unit]);
            }
            image.truncate(len.max(state.durable.len()));
            Some(image)
        })
        .collect();
    (images, acknowledged)
}

/// Every acknowledged chunk read back from the crashed `images`.
fn check(
    recorder: &Recorder,
    images: &[Option<Vec<u8>>],
    acknowledged: &HashMap<(i32, i32), u32>,
    scratch: &Path,
) -> io::Result<Outcome> {
    let mut outcome = Outcome::default();
    let mut regions = HashMap::new();
    for ((path, _), image) in recorder.files.iter().zip(images) {
        let copy = scratch.join(path.file_name().unwrap());
        let _ = fs::remove_file(&copy);
        if let Some(image) = image {
            fs::write(&copy, image)?;
            if let Ok(region) = RegionFile::open(&copy) {
                regions.insert(region.position(), region);
            }
        }
    }
    for (&(x, z), &version) in acknowledged {
        let pos = ChunkPos::new(x, z);
        let read = regions
            .get_mut(&ChunkPos::new(x >> 5, z >> 5))
            .and_then(|region| region.read(pos).ok().flatten())
            .and_then(|record| record.nbt(Limits::default()).ok());
        let field = |tag: &NamedTag, name| match tag.tag.get(name) {
            Some(Tag::Int(v)) => Some(*v),
            _ => None,
        };
        let intact = |tag: &NamedTag, read: u32| match tag.tag.get("filler") {
            Some(Tag::ByteArray(bytes)) => recorder.payloads.get(&((x, z), read)) == Some(bytes),
            _ => false,
        };
        match read {
            Some(tag) if (field(&tag, "xPos"), field(&tag, "zPos")) != (Some(x), Some(z)) => {
                outcome.wrong_key += 1
            }
            Some(tag)
                if field(&tag, "version")
                    .is_some_and(|v| v as u32 >= version && intact(&tag, v as u32)) => {}
            _ => outcome.lost += 1,
        }
    }
    Ok(outcome)
}

/// Refuses a workload whose first rounds would pick from no chunks.
fn check_bounds(chunks: usize) -> io::Result<()> {
    if chunks < 2 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "--chunks must be at least 2: the first half of the rounds writes half of them",
        ));
    }
    Ok(())
}

/// The workload: `rounds` batches written and synced region by region, as `SaveDir` does.
/// With `sync_folder`, the folder is synced after each region file is made, which `SaveDir` doesn't do.
fn workload(
    dir: &Path,
    seed: u64,
    (chunks, batch, rounds): (usize, usize, usize),
    sync_folder: bool,
) -> io::Result<Recorder> {
    check_bounds(chunks)?;
    let mut rng = Rng(seed);
    let mut recorder = Recorder::default();
    let mut versions: HashMap<(i32, i32), u32> = HashMap::new();
    let mut regions: BTreeMap<(i32, i32), RegionFile> = BTreeMap::new();
    for round in 0..rounds {
        // The second region (x 32..64) joins halfway through.
        let pool = chunks / (1 + usize::from(round < rounds / 2));
        let mut picked: BTreeMap<(i32, i32), Vec<(i32, i32)>> = BTreeMap::new();
        for _ in 0..batch {
            let i = rng.below(pool) as i32;
            let key = (
                i % 16 + if i as usize >= chunks / 2 { 32 } else { 0 },
                i / 16 % 16,
            );
            picked
                .entry((key.0 >> 5, key.1 >> 5))
                .or_default()
                .push(key);
        }
        let mut done = Vec::new();
        for ((rx, rz), keys) in picked {
            let path = dir.join(format!("r.{rx}.{rz}.mca"));
            let region = match regions.entry((rx, rz)) {
                Entry::Occupied(region) => region.into_mut(),
                Entry::Vacant(slot) => {
                    let region = slot.insert(RegionFile::open_writable(&path, true)?);
                    recorder.changed(&path)?;
                    if sync_folder {
                        horizon_world::fs_sync::sync_dir(dir)?;
                        recorder.folder_synced();
                    }
                    region
                }
            };
            for key in keys {
                let version = versions.entry(key).or_default();
                *version += 1;
                // From under one sector to about five: records move as they grow and shrink.
                let filler: Vec<u8> = (0..300 + rng.below(19_000))
                    .map(|_| rng.next() as u8)
                    .collect();
                let root = NamedTag {
                    name: "".into(),
                    tag: Tag::compound([
                        ("xPos", Tag::Int(key.0)),
                        ("zPos", Tag::Int(key.1)),
                        ("version", Tag::Int(*version as i32)),
                        ("filler", Tag::ByteArray(filler.clone())),
                    ]),
                };
                recorder.payloads.insert((key, *version), filler);
                let record = ChunkRecord::from_nbt(&root, Compression::None, 0, Limits::default())?;
                region.write(ChunkPos::new(key.0, key.1), &record)?;
                recorder.changed(&path)?;
                done.push((key, *version));
            }
            region.sync()?;
            recorder.synced(&path);
        }
        recorder.acknowledged(done);
    }
    Ok(recorder)
}

fn flag(args: &[String], name: &str, default: u64) -> u64 {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1)?.parse().ok())
        .unwrap_or(default)
}

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let seed = flag(&args, "--seed", 1);
    let cuts = flag(&args, "--cuts", 2000) as usize;
    let (chunks, batch, rounds) = (
        flag(&args, "--chunks", 96) as usize,
        flag(&args, "--batch", 24) as usize,
        flag(&args, "--rounds", 12) as usize,
    );
    check_bounds(chunks)?;
    let root = Scratch::new("horizon-power-cut")?;
    let (work, scratch) = (root.join("work"), root.join("crashed"));
    fs::create_dir(&work)?;
    fs::create_dir(&scratch)?;
    let sync_folder = args.iter().any(|a| a == "--sync-folder");
    let recorder = workload(&work, seed, (chunks, batch, rounds), sync_folder)?;
    let calls = recorder.events.len();
    println!(
        "seed {seed}: {rounds} rounds of {batch} writes over {chunks} chunks, {calls} calls, {cuts} cuts each"
    );
    println!("model   cuts_with_loss  lost_acknowledged  wrong_key_reads");
    for (name, model) in [("ordered", Model::Ordered), ("pages", Model::Pages)]
        .into_iter()
        .chain([("torn", Model::Torn), ("dir", Model::Dir)])
    {
        let mut rng = Rng(seed ^ 0x5EED);
        let (mut bad, mut total) = (0, Outcome::default());
        for _ in 0..cuts {
            let cut = rng.below(recorder.events.len() + 1);
            let (images, acknowledged) = crash(&recorder, cut, model, &mut rng);
            let outcome = check(&recorder, &images, &acknowledged, &scratch)?;
            bad += usize::from(outcome != Outcome::default());
            total.lost += outcome.lost;
            total.wrong_key += outcome.wrong_key;
        }
        println!(
            "{name:<7} {bad:>14} {:>18} {:>16}",
            total.lost, total.wrong_key
        );
    }
    drop(root);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(seed: u64) -> (Recorder, Scratch) {
        let dir = Scratch::new(&format!("horizon-power-cut-test-{seed}")).unwrap();
        fs::create_dir(dir.join("work")).unwrap();
        fs::create_dir(dir.join("crashed")).unwrap();
        let recorder = workload(&dir.join("work"), seed, (32, 8, 4), seed % 2 == 0).unwrap();
        (recorder, dir)
    }

    #[test]
    fn nothing_is_lost_after_the_last_sync() {
        let (recorder, dir) = run(8);
        for model in [Model::Pages, Model::Torn, Model::Dir] {
            let mut rng = Rng(3);
            let (images, acknowledged) = crash(&recorder, recorder.events.len(), model, &mut rng);
            assert!(!acknowledged.is_empty());
            let outcome = check(&recorder, &images, &acknowledged, &dir.join("crashed")).unwrap();
            assert_eq!(outcome, Outcome::default());
        }
    }

    #[test]
    fn writes_kept_in_order_lose_nothing_at_any_cut() {
        let (recorder, dir) = run(9);
        for cut in 0..=recorder.events.len() {
            let (images, acknowledged) = crash(&recorder, cut, Model::Ordered, &mut Rng(1));
            let outcome = check(&recorder, &images, &acknowledged, &dir.join("crashed")).unwrap();
            assert_eq!(outcome, Outcome::default(), "cut after {cut} calls");
        }
    }

    /// A record whose identity fields survive but one interior 512-byte
    /// sector of its filler doesn't still decodes: `check` counts it lost.
    #[test]
    fn a_damaged_payload_sector_counts_as_lost() {
        let (recorder, dir) = run(12);
        let (mut images, acknowledged) = crash(
            &recorder,
            recorder.events.len(),
            Model::Ordered,
            &mut Rng(1),
        );
        let crashed = dir.join("crashed");
        assert_eq!(
            check(&recorder, &images, &acknowledged, &crashed).unwrap(),
            Outcome::default()
        );
        // The first acknowledged chunk of the first region with a record of
        // at least four sectors: its third sector is filler.
        let image = images[0].as_mut().unwrap();
        let record = (0..1024)
            .map(|i| u32::from_be_bytes(image[i * 4..i * 4 + 4].try_into().unwrap()))
            .filter(|&offset| offset != 0)
            .map(|offset| (offset >> 8) as usize * 4096)
            .find(|&at| u32::from_be_bytes(image[at..at + 4].try_into().unwrap()) >= 4 * 512)
            .expect("a record of four sectors or more");
        image[record + 2 * 512..record + 3 * 512].fill(0);
        assert_eq!(
            check(&recorder, &images, &acknowledged, &crashed).unwrap(),
            Outcome {
                lost: 1,
                wrong_key: 0
            }
        );
    }

    #[test]
    fn workloads_without_chunks_to_pick_are_refused() {
        let dir = std::env::temp_dir().join(format!(
            "horizon-power-cut-test-{}-bounds",
            std::process::id()
        ));
        for chunks in [0, 1] {
            let error = workload(&dir, 1, (chunks, 8, 4), false).err().unwrap();
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        }
        assert!(!dir.exists());
    }

    #[test]
    fn the_same_seed_cuts_the_same_way() {
        let (recorder, dir) = run(11);
        let outcomes: Vec<_> = (0..2)
            .map(|_| {
                let mut rng = Rng(5);
                (0..50)
                    .map(|_| {
                        let cut = rng.below(recorder.events.len() + 1);
                        let (images, acknowledged) = crash(&recorder, cut, Model::Torn, &mut rng);
                        let o =
                            check(&recorder, &images, &acknowledged, &dir.join("crashed")).unwrap();
                        (o.lost, o.wrong_key)
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        assert_eq!(outcomes[0], outcomes[1]);
    }
}
