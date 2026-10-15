//! Differential save test: load every full chunk of a world written by the
//! vanilla 26.3 server, light it, encode it again with Horizon's chunk writer
//! and compare the result with the original NBT: block states, biomes,
//! heightmaps, light layers (by value), block entities and scheduled ticks.
//!
//! cargo run --profile fast -p horizon-client --example save_parity -- \
//!     RESOURCES DIMENSION_DIR [--nether]
use horizon_client::{
    assets::Assets,
    biome::Biomes,
    blocks::BlockFacts,
    chunk::{Chunk, MIN_Y, SECTIONS},
    light::{LightTables, apply_lighting, light_chunk},
    save::{self, SaveTables},
};
use horizon_core::position::ChunkPos;
use horizon_world::{
    chunk::BlockState,
    nbt::{Limits, NamedTag, Tag},
    region::RegionFile,
};
use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
};

/// Block state names per section index, decoded independently of Horizon.
fn block_names(tag: &Tag) -> BTreeMap<i64, Vec<String>> {
    let mut out = BTreeMap::new();
    let Some(Tag::List(sections)) = tag.get("sections") else {
        return out;
    };
    for s in sections {
        let Some(Tag::Byte(y)) = s.get("Y") else {
            continue;
        };
        let Some(states) = s.get("block_states") else {
            continue;
        };
        let Some(Tag::List(palette)) = states.get("palette") else {
            continue;
        };
        let names: Vec<String> = palette
            .iter()
            .map(|p| {
                let st = BlockState::from_nbt(p).expect("palette entry");
                format!("{}{:?}", st.name, st.properties)
            })
            .collect();
        let bits = (usize::BITS - (names.len().max(1) - 1).leading_zeros()).max(4);
        let per = 64 / bits as usize;
        let data = match states.get("data") {
            Some(Tag::LongArray(d)) => d.clone(),
            _ => Vec::new(),
        };
        let blocks = (0..4096)
            .map(|i| {
                if names.len() == 1 {
                    return names[0].clone();
                }
                let v = (data[i / per] as u64 >> ((i % per) as u32 * bits)) & ((1 << bits) - 1);
                names[v as usize].clone()
            })
            .collect();
        out.insert(i64::from(*y), blocks);
    }
    out
}
fn light(tag: &Tag, key: &str) -> HashMap<i64, Vec<u8>> {
    let mut out = HashMap::new();
    if let Some(Tag::List(sections)) = tag.get("sections") {
        for s in sections {
            if let (Some(Tag::Byte(y)), Some(Tag::ByteArray(a))) = (s.get("Y"), s.get(key)) {
                out.insert(i64::from(*y), a.clone());
            }
        }
    }
    out
}
/// Light value with vanilla's reading of missing layers.
fn value(
    layers: &HashMap<i64, Vec<u8>>,
    sy: i64,
    i: usize,
    sky: bool,
    empty: bool,
    top: i64,
) -> u8 {
    let nibble = |a: &Vec<u8>, i: usize| (a[i >> 1] >> ((i & 1) * 4)) & 15;
    if let Some(a) = layers.get(&sy) {
        return nibble(a, i);
    }
    if !sky || !empty {
        return 0;
    }
    ((sy + 1)..=top)
        .find_map(|y| layers.get(&y))
        .map_or(15, |a| nibble(a, i & 255))
}
fn count(tag: &Tag, key: &str) -> usize {
    match tag.get(key) {
        Some(Tag::List(l)) => l.len(),
        _ => 0,
    }
}

fn main() -> horizon_client::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = PathBuf::from(
        args.first()
            .ok_or("usage: save_parity RESOURCES DIMENSION_DIR")?,
    );
    let dim = PathBuf::from(args.get(1).ok_or("missing dimension directory")?);
    let sky_light = !args.iter().any(|a| a == "--nether");
    let assets = Assets::load(&root)?;
    let facts = BlockFacts::load(&root, assets.registry)?;
    let biomes = Biomes::load(&root)?;
    let lights = LightTables::load(&root, assets.registry, &facts)?;
    let tables = SaveTables::new(&assets).with_block_entities(&facts);
    let mut originals: HashMap<(i32, i32), NamedTag> = HashMap::new();
    let mut chunks: HashMap<(i32, i32), Chunk> = HashMap::new();
    for entry in std::fs::read_dir(dim.join("region"))? {
        let mut region = RegionFile::open(entry?.path())?;
        let positions: Vec<ChunkPos> = region.chunks().map(|(p, _)| p).collect();
        for p in positions {
            let Some(record) = region.read(p)? else {
                continue;
            };
            let tag = record.nbt(Limits::default())?;
            if tag.tag.get("isLightOn") != Some(&Tag::Byte(1)) {
                continue;
            }
            if let Some(c) = save::decode_with(&tag, assets.registry, &biomes, Some(&tables)) {
                chunks.insert((c.x, c.z), c);
                originals.insert((p.x, p.z), tag);
            }
        }
    }
    let mut keys: Vec<_> = chunks
        .keys()
        .copied()
        .filter(|&(x, z)| {
            (-1..=1).all(|dz| (-1..=1).all(|dx| chunks.contains_key(&(x + dx, z + dz))))
        })
        .collect();
    keys.sort_unstable();
    let (
        mut blocks_bad,
        mut biomes_bad,
        mut heightmaps_bad,
        mut light_bad,
        mut light_total,
        mut other_bad,
    ) = (0, 0, 0, 0u64, 0u64, 0);
    for &(x, z) in &keys {
        let region: [[Option<&Chunk>; 3]; 3] = std::array::from_fn(|dz| {
            std::array::from_fn(|dx| chunks.get(&(x + dx as i32 - 1, z + dz as i32 - 1)))
        });
        let mut chunk = chunks[&(x, z)].clone();
        apply_lighting(&mut chunk, light_chunk(&region, &lights, sky_light));
        chunk.lit = true;
        let ours = save::encode_with(&chunk, assets.registry, &biomes, Some(&tables), sky_light);
        let theirs = &originals[&(x, z)];
        let (a, b) = (&ours.tag, &theirs.tag);
        // Block states: compare every non-empty vanilla section.
        let (na, nb) = (block_names(a), block_names(b));
        for (y, vb) in &nb {
            match na.get(y) {
                Some(va) if va == vb => {}
                Some(va) => {
                    let n = va.iter().zip(vb).filter(|(p, q)| p != q).count();
                    if blocks_bad < 5 {
                        let i = va.iter().zip(vb).position(|(p, q)| p != q).unwrap_or(0);
                        eprintln!(
                            "chunk {x} {z} section {y}: {n} blocks differ, e.g. {} vs {}",
                            va[i], vb[i]
                        );
                    }
                    blocks_bad += 1;
                }
                None if vb.iter().all(|s| s.starts_with("minecraft:air")) => {}
                None => blocks_bad += 1,
            }
        }
        let biome_of = |t: &Tag| -> Vec<Tag> {
            match t.get("sections") {
                Some(Tag::List(l)) => l.iter().filter_map(|s| s.get("biomes").cloned()).collect(),
                _ => Vec::new(),
            }
        };
        if biome_of(a) != biome_of(b) {
            biomes_bad += 1;
        }
        for kind in [
            "WORLD_SURFACE",
            "OCEAN_FLOOR",
            "MOTION_BLOCKING",
            "MOTION_BLOCKING_NO_LEAVES",
        ] {
            let get = |t: &Tag| t.get("Heightmaps").and_then(|h| h.get(kind)).cloned();
            if get(a) != get(b) {
                if heightmaps_bad < 5 {
                    eprintln!("chunk {x} {z}: heightmap {kind} differs");
                }
                heightmaps_bad += 1;
            }
        }
        for (key, sky) in [("SkyLight", true), ("BlockLight", false)] {
            if sky && !sky_light {
                continue;
            }
            let (la, lb) = (light(a, key), light(b, key));
            let top = i64::from((MIN_Y >> 4) + SECTIONS as i32);
            for si in 0..SECTIONS {
                let sy = i64::from((MIN_Y >> 4) + si as i32);
                let empty = chunk.sections[si].is_uniform(tables_air(&assets));
                for i in 0..4096 {
                    light_total += 1;
                    if value(&la, sy, i, sky, empty, top) != value(&lb, sy, i, sky, empty, top) {
                        light_bad += 1;
                    }
                }
            }
        }
        for key in ["block_entities", "block_ticks", "fluid_ticks"] {
            if count(a, key) != count(b, key) {
                if other_bad < 5 {
                    eprintln!(
                        "chunk {x} {z}: {key} {} vs {}",
                        count(a, key),
                        count(b, key)
                    );
                }
                other_bad += 1;
            }
        }
        for key in [
            "structures",
            "PostProcessing",
            "InhabitedTime",
            "Status",
            "DataVersion",
            "yPos",
        ] {
            if a.get(key) != b.get(key) {
                if other_bad < 5 {
                    eprintln!("chunk {x} {z}: {key} differs");
                }
                other_bad += 1;
            }
        }
    }
    println!(
        "{} chunks: {blocks_bad} sections with different blocks, {biomes_bad} chunks with different biomes, \
         {heightmaps_bad} different heightmaps, {light_bad} of {light_total} light values differ, {other_bad} other differences",
        keys.len()
    );
    Ok(())
}
fn tables_air(assets: &Assets) -> u16 {
    assets
        .registry
        .default_state("minecraft:air")
        .map_or(0, |s| s.0 as u16)
}
