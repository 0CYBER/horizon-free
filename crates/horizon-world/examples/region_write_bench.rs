//! Run against a scratch directory on the filesystem to measure:
//! `cargo run --profile fast -p horizon-world --example region_write_bench -- <scratch-dir>`.
//! Timings exclude fixture compression and hashing. The digest covers the whole published region.
use horizon_core::{
    nbt::{Limits, NamedTag, Tag},
    position::ChunkPos,
};
use horizon_world::{
    compression::Compression,
    region::{ChunkRecord, RegionWriter},
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{env, fs, io, path::PathBuf, time::Instant};

fn main() -> io::Result<()> {
    const CHUNKS: usize = 512;
    let directory = env::args_os().nth(1).map(PathBuf::from).ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "expected a scratch directory")
    })?;
    let path = directory.join("r.0.0.mca");
    let records = (0..CHUNKS as i32)
        .map(|i| {
            let x = i % 32;
            let z = i / 32;
            let tag = NamedTag {
                name: "".into(),
                tag: Tag::compound([("xPos", Tag::Int(x)), ("zPos", Tag::Int(z))]),
            };
            ChunkRecord::from_nbt(&tag, Compression::Zlib, i as u32 + 1, Limits::default())
                .map(|record| (ChunkPos::new(x, z), record))
        })
        .collect::<io::Result<Vec<_>>>()?;

    let started = Instant::now();
    let mut writer = RegionWriter::create(&path)?;
    let write_started = Instant::now();
    for (pos, record) in &records {
        writer.write(*pos, record)?;
    }
    let finish_started = Instant::now();
    writer.finish()?;
    let ended = Instant::now();

    let sha256 = format!("{:x}", Sha256::digest(fs::read(&path)?));
    fs::remove_file(path)?;
    println!(
        "{}",
        json!({
            "chunks": CHUNKS,
            "ns_per_chunk": finish_started.duration_since(write_started).as_nanos() / CHUNKS as u128,
            "finish_ns": ended.duration_since(finish_started).as_nanos(),
            "total_ns": ended.duration_since(started).as_nanos(),
            "sha256": sha256,
        })
    );
    Ok(())
}
