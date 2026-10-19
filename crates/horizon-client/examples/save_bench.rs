//! The region writer on real chunks, without a window: how long a world's
//! chunks take to be encoded, compressed and written by `SaveDir`, against
//! the same encoding and compression done one chunk after another (what the
//! writer thread did alone before it had helpers).
//!
//! save_bench RESOURCES DIMENSION_DIR [--rounds N]
//!
//! DIMENSION_DIR is a saved dimension folder (`saves/W/dimensions/minecraft/
//! overworld`); its chunks are read once, then each round saves them all into
//! a scratch folder with `save_later` + `flush`, and times the one-thread
//! baseline. The written chunks are checked byte for byte against the
//! baseline's compressed records.
use horizon_client::{
    assets::Assets,
    biome::Biomes,
    blocks::BlockFacts,
    save::{SaveDir, SaveTables, encode_with},
};
use horizon_core::position::ChunkPos;
use horizon_world::{
    compression::Compression,
    nbt::Limits,
    region::{ChunkRecord, RegionFile},
};
use std::{path::PathBuf, sync::Arc, time::Instant};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let usage = "usage: save_bench RESOURCES DIMENSION_DIR [--rounds N]";
    let root = PathBuf::from(args.first().ok_or(usage)?);
    let dimension = PathBuf::from(args.get(1).ok_or(usage)?);
    let rounds: usize = args
        .iter()
        .position(|a| a == "--rounds")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(5);

    let assets = Assets::load(&root)?;
    let facts = BlockFacts::load(&root, assets.registry)?;
    let biomes = Arc::new(Biomes::load(&root)?);
    let tables = Arc::new(SaveTables::new(&assets).with_block_entities(&facts));
    let registry = assets.registry;

    let source = SaveDir::new(dimension.clone());
    let _ = source.tables.set(tables.clone());
    let mut chunks = Vec::new();
    for entry in std::fs::read_dir(dimension.join("region"))? {
        let path = entry?.path();
        if path.extension().is_none_or(|e| e != "mca") {
            continue;
        }
        let region = RegionFile::open(&path)?;
        for (pos, _) in region.chunks() {
            if let Some(chunk) = source.load(pos.x, pos.z, registry, &biomes) {
                chunks.push(Arc::new(chunk));
            }
        }
    }
    chunks.sort_by_key(|c| (c.x, c.z));
    println!("{} chunks from {}", chunks.len(), dimension.display());

    let scratch = std::env::temp_dir().join(format!("save_bench_{}", std::process::id()));
    // Encoding and compressing on one thread (what the writer did alone),
    // then SaveDir's writer with its helpers.
    let mut times = [Vec::new(), Vec::new(), Vec::new()];
    let mut written = Vec::new();
    for round in 0..rounds {
        let started = Instant::now();
        let tags: Vec<_> = chunks
            .iter()
            .map(|c| encode_with(c, registry, &biomes, Some(&tables), true))
            .collect();
        times[0].push(started.elapsed().as_secs_f64());
        let started = Instant::now();
        let baseline: Vec<ChunkRecord> = tags
            .iter()
            .map(|tag| ChunkRecord::from_nbt(tag, Compression::Zlib, 0, Limits::default()))
            .collect::<std::io::Result<_>>()?;
        times[1].push(started.elapsed().as_secs_f64());
        drop(tags);

        let _ = std::fs::remove_dir_all(&scratch);
        let save = SaveDir::new(scratch.clone());
        let _ = save.tables.set(tables.clone());
        let started = Instant::now();
        save.save_later(&chunks, registry, &biomes)?;
        save.flush().unwrap();
        times[2].push(started.elapsed().as_secs_f64());
        drop(save);

        if round == 0 {
            for (chunk, want) in chunks.iter().zip(&baseline) {
                let path = scratch.join(format!("region/r.{}.{}.mca", chunk.x >> 5, chunk.z >> 5));
                let got = RegionFile::open(&path)?
                    .read(ChunkPos::new(chunk.x, chunk.z))?
                    .ok_or("a saved chunk is missing")?;
                if got.compressed != want.compressed {
                    return Err(format!("chunk {} {} differs", chunk.x, chunk.z).into());
                }
            }
            written = baseline.iter().map(|r| r.compressed.len()).collect();
        }
    }
    let _ = std::fs::remove_dir_all(&scratch);
    println!(
        "{} KiB compressed, identical to the one-thread records",
        written.iter().sum::<usize>() / 1024
    );
    let per_chunk = 1000.0 / chunks.len() as f64;
    let names = [
        "one thread: encode",
        "one thread: compress",
        "SaveDir (encode, compress, write)",
    ];
    for (name, t) in names.iter().zip(&mut times) {
        t.sort_by(f64::total_cmp);
        println!(
            "{name}: best {:.0} ms, median {:.0} ms of {rounds} ({:.3} ms per chunk at best)",
            t[0] * 1000.0,
            t[t.len() / 2] * 1000.0,
            t[0] * per_chunk
        );
    }
    Ok(())
}
