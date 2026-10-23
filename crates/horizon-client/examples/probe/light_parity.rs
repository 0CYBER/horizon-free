//! Differential light test: relight chunks of a world saved by the vanilla
//! server with Horizon's light engine and compare every SkyLight/BlockLight
//! value with the arrays vanilla stored.
//!
//! cargo run --profile fast -p horizon-client --example probe -- light_parity \
//!     RESOURCES DIMENSION_DIR [--nether] [--chunks X0 Z0 X1 Z1]
use horizon_client::{
    assets::Assets,
    biome::Biomes,
    blocks::BlockFacts,
    chunk::{Chunk, MIN_Y, SECTIONS, index},
    light::{LightTables, apply_lighting, light_chunk},
    save,
};
use horizon_core::position::ChunkPos;
use horizon_world::{
    nbt::{Limits, Tag},
    region::RegionFile,
};
use std::{collections::HashMap, path::PathBuf};

type Arrays = HashMap<i32, (Option<Vec<u8>>, Option<Vec<u8>>)>;

pub fn main() -> horizon_client::Result<()> {
    let args: Vec<String> = crate::args().skip(1).collect();
    let root = PathBuf::from(
        args.first()
            .ok_or("usage: light_parity RESOURCES DIMENSION_DIR")?,
    );
    let dim = PathBuf::from(args.get(1).ok_or("missing dimension directory")?);
    let sky_light = !args.iter().any(|a| a == "--nether");
    let assets = Assets::load(&root)?;
    let facts = BlockFacts::load(&root, assets.registry)?;
    let biomes = Biomes::load(&root)?;
    let tables = if args.iter().any(|a| a == "--no-shapes") {
        LightTables::new(&facts)
    } else {
        LightTables::load(&root, assets.registry, &facts)?
    };
    let mut chunks: HashMap<(i32, i32), Chunk> = HashMap::new();
    let mut arrays: HashMap<(i32, i32), Arrays> = HashMap::new();
    for entry in std::fs::read_dir(dim.join("region"))? {
        let path = entry?.path();
        let mut region = RegionFile::open(&path)?;
        let positions: Vec<ChunkPos> = region.chunks().map(|(p, _)| p).collect();
        for p in positions {
            let Some(record) = region.read(p)? else {
                continue;
            };
            let root_tag = record.nbt(Limits::default())?;
            let tag = &root_tag.tag;
            if !matches!(tag.get("Status"), Some(Tag::String(s)) if s.to_string_lossy() == "minecraft:full")
            {
                continue;
            }
            if tag.get("isLightOn") != Some(&Tag::Byte(1)) {
                continue;
            }
            let Some(chunk) = save::decode(&root_tag, assets.registry, &biomes) else {
                continue;
            };
            let mut light = Arrays::new();
            if let Some(Tag::List(sections)) = tag.get("sections") {
                for s in sections {
                    let Some(Tag::Byte(y)) = s.get("Y") else {
                        continue;
                    };
                    let get = |k: &str| match s.get(k) {
                        Some(Tag::ByteArray(a)) => Some(a.clone()),
                        _ => None,
                    };
                    light.insert(i32::from(*y), (get("SkyLight"), get("BlockLight")));
                }
            }
            arrays.insert((chunk.x, chunk.z), light);
            chunks.insert((chunk.x, chunk.z), chunk);
        }
    }
    let bounds: Option<[i32; 4]> = args.iter().position(|a| a == "--chunks").map(|i| {
        std::array::from_fn(|k| {
            args.get(i + 1 + k)
                .and_then(|v| v.parse().ok())
                .unwrap_or(0)
        })
    });
    if let Some(i) = args.iter().position(|a| a == "--column") {
        let wx: i32 = args[i + 1].parse()?;
        let wz: i32 = args[i + 2].parse()?;
        let (y0, y1): (i32, i32) = (args[i + 3].parse()?, args[i + 4].parse()?);
        let (x, z) = (wx >> 4, wz >> 4);
        let region: [[Option<&Chunk>; 3]; 3] = std::array::from_fn(|dz| {
            std::array::from_fn(|dx| chunks.get(&(x + dx as i32 - 1, z + dz as i32 - 1)))
        });
        let lit = light_chunk(&region, &tables, sky_light);
        for wy in (y0..=y1).rev() {
            let si = ((wy - MIN_Y) >> 4) as usize;
            let i = index(wx, wy, wz);
            let (sky, block) = arrays[&(x, z)]
                .get(&((wy) >> 4))
                .cloned()
                .unwrap_or((None, None));
            let nib = |a: &Option<Vec<u8>>| a.as_ref().map(|a| (a[i >> 1] >> ((i & 1) * 4)) & 15);
            let id = chunks[&(x, z)].get(wx & 15, wy, wz & 15);
            let packed = lit[si].get(i);
            println!(
                "{wy}: {} horizon sky {} block {} vanilla sky {:?} block {:?}",
                facts.states[id as usize].name,
                packed >> 4,
                packed & 15,
                nib(&sky),
                nib(&block)
            );
        }
        return Ok(());
    }
    if let Some(i) = args.iter().position(|a| a == "--incremental") {
        let edits: usize = args.get(i + 1).and_then(|v| v.parse().ok()).unwrap_or(200);
        incremental(&chunks, &tables, &facts, assets.registry, sky_light, edits);
        return Ok(());
    }
    let mut keys: Vec<(i32, i32)> = chunks
        .keys()
        .copied()
        .filter(|&(x, z)| {
            (-1..=1).all(|dz| (-1..=1).all(|dx| chunks.contains_key(&(x + dx, z + dz))))
        })
        .filter(|&(x, z)| bounds.is_none_or(|b| x >= b[0] && z >= b[1] && x <= b[2] && z <= b[3]))
        .collect();
    keys.sort_unstable();
    let (mut total, mut sky_wrong, mut block_wrong) = (0u64, 0u64, 0u64);
    let mut shown = 0;
    for (x, z) in keys {
        let region: [[Option<&Chunk>; 3]; 3] = std::array::from_fn(|dz| {
            std::array::from_fn(|dx| chunks.get(&(x + dx as i32 - 1, z + dz as i32 - 1)))
        });
        let lit = light_chunk(&region, &tables, sky_light);
        let vanilla = &arrays[&(x, z)];
        let center = &chunks[&(x, z)];
        let (mut s_bad, mut b_bad) = (0, 0);
        for (si, section) in lit.iter().enumerate().take(SECTIONS) {
            let sy = (MIN_Y >> 4) + si as i32;
            let (sky, block) = vanilla.get(&sy).cloned().unwrap_or((None, None));
            for i in 0..4096 {
                let nibble = |a: &Option<Vec<u8>>, default: u8| {
                    a.as_ref()
                        .map_or(default, |a| (a[i >> 1] >> ((i & 1) * 4)) & 15)
                };
                // Vanilla omits never-written (all-zero) layers. Air sections
                // without a layer read the bottom row of the next stored layer
                // above (SkyLightSectionStorage.getLightValue), or full
                // skylight above every stored layer.
                let vs = match &sky {
                    Some(_) => nibble(&sky, 0),
                    None if !sky_light || !center.sections[si].is_uniform(0) => 0,
                    None => ((sy + 1)..=(MIN_Y >> 4) + SECTIONS as i32)
                        .find_map(|y| vanilla.get(&y).and_then(|(s, _)| s.as_ref()))
                        .map_or(15, |a| (a[(i & 255) >> 1] >> ((i & 1) * 4)) & 15),
                };
                let vb = nibble(&block, 0);
                let packed = section.get(i);
                let (hs, hb) = (packed >> 4, packed & 15);
                total += 1;
                if hs != vs {
                    s_bad += 1;
                }
                if hb != vb {
                    b_bad += 1;
                }
                if (hs != vs || hb != vb) && shown < 30 {
                    shown += 1;
                    let (lx, lz, ly) = (i & 15, (i >> 4) & 15, i >> 8);
                    let wy = sy * 16 + ly as i32;
                    let wx = x * 16 + lx as i32;
                    let wz = z * 16 + lz as i32;
                    let id = center.get(lx as i32, wy, lz as i32);
                    eprintln!(
                        "  {wx} {wy} {wz} {}: sky {hs} vs {vs}, block {hb} vs {vb}",
                        facts.states[id as usize].name
                    );
                }
                let _ = index;
            }
        }
        if s_bad + b_bad > 0 {
            println!("chunk {x} {z}: {s_bad} sky and {b_bad} block values differ");
        }
        sky_wrong += s_bad;
        block_wrong += b_bad;
    }
    println!(
        "{total} values compared: {sky_wrong} sky and {block_wrong} block light values differ"
    );
    if sky_wrong + block_wrong > 0 {
        std::process::exit(1);
    }
    Ok(())
}

/// Edit lit chunks block by block with `light::relight` and check the result
/// against a full relight of the edited neighbourhood after every edit.
fn incremental(
    chunks: &HashMap<(i32, i32), Chunk>,
    tables: &LightTables,
    facts: &BlockFacts,
    registry: &horizon_world::registry::BlockRegistry,
    sky_light: bool,
    edits: usize,
) {
    use horizon_client::light::{LightChanges, relight};
    use std::sync::Arc;
    let mut world: HashMap<(i32, i32), Arc<Chunk>> = HashMap::new();
    let inner: Vec<(i32, i32)> = {
        let mut k: Vec<_> = chunks
            .keys()
            .copied()
            .filter(|&(x, z)| {
                (-3..=3).all(|dz| (-3..=3).all(|dx| chunks.contains_key(&(x + dx, z + dz))))
            })
            .collect();
        k.sort_unstable();
        k
    };
    for &(x, z) in chunks.keys() {
        if !(-1..=1).all(|dz| (-1..=1).all(|dx| chunks.contains_key(&(x + dx, z + dz)))) {
            continue;
        }
        let region: [[Option<&Chunk>; 3]; 3] = std::array::from_fn(|dz| {
            std::array::from_fn(|dx| chunks.get(&(x + dx as i32 - 1, z + dz as i32 - 1)))
        });
        let mut c = chunks[&(x, z)].clone();
        apply_lighting(&mut c, light_chunk(&region, tables, sky_light));
        c.lit = true;
        world.insert((x, z), Arc::new(c));
    }
    let palette: Vec<u32> = [
        "minecraft:air",
        "minecraft:stone",
        "minecraft:torch",
        "minecraft:glowstone",
        "minecraft:oak_leaves",
        "minecraft:water",
        "minecraft:glass",
    ]
    .iter()
    .filter_map(|n| registry.default_state(n).map(|s| s.0))
    .chain(
        facts
            .states
            .iter()
            .enumerate()
            .filter(|(_, f)| {
                [
                    "minecraft:stone_slab",
                    "minecraft:oak_stairs",
                    "minecraft:snow",
                    "minecraft:dirt_path",
                ]
                .contains(&&*f.name)
            })
            .map(|(i, _)| i as u32)
            .step_by(3),
    )
    .collect();
    let mut seed = 0x2545_F491_4F6C_DD1Du64;
    let mut next = |n: u64| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed % n
    };
    let mut bad = 0;
    for e in 0..edits {
        let (cx, cz) = inner[next(inner.len() as u64) as usize];
        let (x, z) = (cx * 16 + next(16) as i32, cz * 16 + next(16) as i32);
        // Near the surface, where light varies most.
        let top = (0..384)
            .rev()
            .find(|y| world[&(cx, cz)].get(x & 15, MIN_Y + y, z & 15) != 0)
            .unwrap_or(100)
            + MIN_Y;
        let y = top - 3 + next(6) as i32;
        let state = palette[next(palette.len() as u64) as usize];
        let chunk = world.get_mut(&(cx, cz)).unwrap();
        let old = Arc::make_mut(chunk).set(x & 15, y, z & 15, state);
        if old == state || !tables.differs(old, state) {
            continue;
        }
        relight(
            &mut world,
            tables,
            sky_light,
            &[(x, y, z)],
            false,
            &mut LightChanges::default(),
        );
        for dz in -1..=1 {
            for dx in -1..=1 {
                let (kx, kz) = (cx + dx, cz + dz);
                let region: [[Option<&Chunk>; 3]; 3] = std::array::from_fn(|rz| {
                    std::array::from_fn(|rx| {
                        world
                            .get(&(kx + rx as i32 - 1, kz + rz as i32 - 1))
                            .map(|c| &**c)
                    })
                });
                let full = light_chunk(&region, tables, sky_light);
                for (si, s) in full.iter().enumerate() {
                    for i in 0..4096 {
                        let (a, b) = (s.get(i), world[&(kx, kz)].sections[si].light(i));
                        if a != b {
                            bad += 1;
                            if bad < 20 {
                                let (lx, lz, ly) = (i & 15, (i >> 4) & 15, i >> 8);
                                eprintln!(
                                    "edit {e} at {x} {y} {z} ({} -> {}): {} {} {} full {a:02x} incremental {b:02x}",
                                    facts.states[old as usize].name,
                                    facts.states[state as usize].name,
                                    kx * 16 + lx as i32,
                                    MIN_Y + si as i32 * 16 + ly as i32,
                                    kz * 16 + lz as i32
                                );
                            }
                        }
                    }
                }
            }
        }
    }
    println!("{edits} edits: {bad} light values differ from a full relight");
    if bad > 0 {
        std::process::exit(1);
    }
}
