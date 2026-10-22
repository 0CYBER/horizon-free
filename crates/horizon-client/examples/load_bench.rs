//! Read-only saved chunk benchmark. All timings exclude resource/corpus setup.
//!
//! load_bench RESOURCES DIMENSION_DIR [--rounds N] [--limit N] [--checksum]
//! Add --profile-worker when a sampling collector cannot sample the main thread.
//! Reports warm filesystem SaveDir::load (read, inflate, NBT, decode and light),
//! plus isolated inflate/NBT and native decode. No files in the save are changed.
use horizon_client::{
    assets::Assets,
    biome::Biomes,
    blocks::BlockFacts,
    save::{SaveDir, SaveTables, decode_with},
};
use horizon_world::{nbt::Limits, region::RegionFile};
use sha2::{Digest, Sha256};
use std::{hint::black_box, path::PathBuf, sync::Arc, time::Instant};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn main() -> Result<()> {
    if std::env::args().any(|a| a == "--profile-worker") {
        std::thread::spawn(|| run().map_err(|e| e.to_string()))
            .join()
            .expect("benchmark worker")
            .map_err(Into::into)
    } else {
        run()
    }
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let usage = "load_bench RESOURCES DIMENSION_DIR [--rounds N] [--limit N] [--checksum]";
    let root = PathBuf::from(args.first().ok_or(usage)?);
    let dimension = PathBuf::from(args.get(1).ok_or(usage)?);
    let flag = |name: &str, default: usize| -> Result<usize> {
        Ok(match args.iter().position(|a| a == name) {
            Some(i) => args.get(i + 1).ok_or(usage)?.parse()?,
            None => default,
        })
    };
    let rounds = flag("--rounds", 5)?;
    let limit = flag("--limit", 256)?;
    if rounds == 0 || limit == 0 {
        return Err("rounds and limit must be positive".into());
    }
    let assets = Assets::load(&root)?;
    let facts = BlockFacts::load(&root, assets.registry)?;
    let biomes = Biomes::load(&root)?;
    let tables = Arc::new(SaveTables::new(&assets).with_block_entities(&facts));
    let source = SaveDir::new(dimension.clone());
    let _ = source.tables.set(tables.clone());
    let mut paths: Vec<_> = std::fs::read_dir(dimension.join("region"))?
        .map(|e| e.map(|e| e.path()))
        .collect::<std::io::Result<_>>()?;
    paths.retain(|p| p.extension().is_some_and(|e| e == "mca"));
    paths.sort();
    let mut corpus = Vec::new();
    'regions: for path in paths {
        let mut region = RegionFile::open(path)?;
        let mut positions: Vec<_> = region.chunks().map(|(pos, _)| pos).collect();
        positions.sort_by_key(|p| (p.x, p.z));
        for pos in positions {
            let record = region.read(pos)?.ok_or("missing record")?;
            let tag = record.nbt(Limits::default())?;
            if decode_with(&tag, assets.registry, &biomes, Some(&tables)).is_none() {
                continue;
            }
            corpus.push((pos, record, tag));
            if corpus.len() == limit {
                break 'regions;
            }
        }
    }
    if corpus.is_empty() {
        return Err("no full chunks in corpus".into());
    }
    let mut records_hash = Sha256::new();
    for (pos, record, _) in &corpus {
        records_hash.update(pos.x.to_le_bytes());
        records_hash.update(pos.z.to_le_bytes());
        records_hash.update((record.compressed.len() as u64).to_le_bytes());
        records_hash.update(&record.compressed);
    }
    println!(
        "{}",
        serde_json::json!({"corpus": dimension, "chunks": corpus.len(), "rounds": rounds,
            "compressed_records_sha256": format!("{:x}", records_hash.finalize()),
            "compressed_bytes": corpus.iter().map(|(_, r, _)| r.compressed.len()).sum::<usize>()})
    );
    // Prime SaveDir's region handles and the filesystem cache before measurement.
    for (pos, _, _) in &corpus {
        black_box(
            source
                .load(pos.x, pos.z, assets.registry, &biomes)
                .ok_or("load failed")?,
        );
    }
    for round in 0..rounds {
        let started = Instant::now();
        for (pos, _, _) in &corpus {
            black_box(
                source
                    .load(pos.x, pos.z, assets.registry, &biomes)
                    .ok_or("load failed")?,
            );
        }
        let load_s = started.elapsed().as_secs_f64();
        let started = Instant::now();
        for (_, record, _) in &corpus {
            black_box(record.nbt(Limits::default())?);
        }
        let nbt_s = started.elapsed().as_secs_f64();
        let started = Instant::now();
        for (_, _, tag) in &corpus {
            black_box(
                decode_with(tag, assets.registry, &biomes, Some(&tables)).ok_or("decode failed")?,
            );
        }
        let decode_s = started.elapsed().as_secs_f64();
        println!(
            "{}",
            serde_json::json!({"round": round, "load_s": load_s,
            "inflate_nbt_s": nbt_s, "decode_s": decode_s})
        );
    }
    if args.iter().any(|a| a == "--checksum") {
        let mut hash = Sha256::new();
        let mut light_loaded = 0;
        for (pos, _, _) in &corpus {
            let chunk = source
                .load(pos.x, pos.z, assets.registry, &biomes)
                .ok_or("load failed")?;
            hash.update(pos.x.to_le_bytes());
            hash.update(pos.z.to_le_bytes());
            light_loaded += usize::from(chunk.light_loaded);
            for section in &chunk.sections {
                for i in 0..4096 {
                    hash.update(section.get(i).to_le_bytes());
                    hash.update([section.light(i)]);
                }
                for i in 0..64 {
                    hash.update(section.biomes.get(i).to_le_bytes());
                }
            }
        }
        println!(
            "{}",
            serde_json::json!({"blocks_biomes_light_sha256": format!("{:x}", hash.finalize()),
            "light_loaded": light_loaded})
        );
    }
    Ok(())
}
