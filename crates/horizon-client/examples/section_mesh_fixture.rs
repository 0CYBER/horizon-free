//! Headless transport for the section-mesh parity gate: builds the requested
//! scenes in real client chunks and writes the vertices `Mesher::mesh` emits in
//! the oracle's canonical form (vanilla ARGB colours and packed light coords).
#![forbid(unsafe_code)]
use horizon_client::{
    assets::Assets,
    biome::{Biomes, GrassModifier},
    blocks::BlockFacts,
    chunk::{Chunk, Light, MIN_Y, index},
    mesher::{MeshTables, Mesher, Vertex},
    models,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    io::{BufRead, BufReader, BufWriter, Write},
    path::PathBuf,
    sync::Arc,
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn int(value: &Value) -> Result<i32> {
    Ok(i32::try_from(value.as_i64().ok_or("expected integer")?)?)
}
/// Horizon packs RGBA little endian (red in the low byte); vanilla uses ARGB.
fn argb(color: u32) -> i32 {
    let [r, g, b, a] = color.to_le_bytes();
    i32::from_be_bytes([a, r, g, b])
}
/// Horizon keeps sky*16 in the low byte and block*16 above it; vanilla's light
/// coords keep block in the low half-word and sky in the high one.
fn light_coords(light: u32) -> i32 {
    ((light & 0xFF) << 16 | (light >> 8 & 0xFF)) as i32
}
/// Scenes cover the 3x3 chunk region around chunk (0, 0).
fn chunk_index(x: i32, z: i32) -> Result<usize> {
    let (cx, cz) = (x.div_euclid(16), z.div_euclid(16));
    if !(-1..=1).contains(&cx) || !(-1..=1).contains(&cz) {
        return Err("scene block outside the 3x3 chunk region".into());
    }
    Ok(((cz + 1) * 3 + cx + 1) as usize)
}
fn scene(
    row: &Value,
    assets: &Assets,
    tables: &MeshTables,
    biomes: &mut Biomes,
    names: &BTreeMap<u32, &str>,
) -> Result<Value> {
    let colors = &row["colors"];
    let color = |k: &str| -> Result<u32> { Ok(int(&colors[k])? as u32 & 0xFF_FFFF) };
    for biome in &mut biomes.list {
        biome.grass = color("grass")?;
        biome.modifier = GrassModifier::None;
        biome.foliage = color("foliage")?;
        biome.dry_foliage = color("dry_foliage")?;
        biome.water = color("water")?;
    }
    let default = row["light"].as_array().ok_or("missing default light")?;
    let packed = |sky: i32, block: i32| (sky << 4 | block) as u8;
    let default = packed(int(&default[0])?, int(&default[1])?);
    let mut chunks: Vec<Chunk> = (0..9)
        .map(|i| {
            let mut chunk = Chunk::empty(i % 3 - 1, i / 3 - 1);
            for section in &mut chunk.sections {
                Arc::make_mut(section).light = Light::Uniform(default);
            }
            chunk
        })
        .collect();
    for block in row["blocks"].as_array().ok_or("missing blocks")? {
        let (x, y, z) = (int(&block[0])?, int(&block[1])?, int(&block[2])?);
        let i = chunk_index(x, z)?;
        chunks[i].set(x & 15, y, z & 15, u16::try_from(int(&block[3])?)?);
    }
    for cell in row["lights"].as_array().ok_or("missing lights")? {
        let (x, y, z) = (int(&cell[0])?, int(&cell[1])?, int(&cell[2])?);
        let i = chunk_index(x, z)?;
        let s = ((y - MIN_Y) >> 4) as usize;
        let section = Arc::make_mut(&mut chunks[i].sections[s]);
        if let Light::Uniform(v) = section.light {
            section.light = Light::Full(Box::new([v; 4096]));
        }
        if let Light::Full(values) = &mut section.light {
            values[index(x, y, z)] = packed(int(&cell[3])?, int(&cell[4])?);
        }
    }
    let region: [[Option<&Chunk>; 3]; 3] =
        std::array::from_fn(|z| std::array::from_fn(|x| Some(&chunks[z * 3 + x])));
    let mut mesher = Mesher {
        nether_shading: row["nether"].as_bool().unwrap_or(false),
        assets,
        tables,
        biomes: &*biomes,
        visibility: tables.visibility.clone(),
    };
    let section_y = int(&row["section_y"])?;
    let mesh = mesher.mesh(&region, (section_y * 16 - MIN_Y) / 16);
    let mut quads = Vec::new();
    for (layer, vertices) in [("opaque", &mesh.solid), ("translucent", &mesh.translucent)] {
        if vertices.len() % 4 != 0 {
            return Err("partial quad".into());
        }
        for quad in vertices.chunks_exact(4) {
            let sprite = names.get(&quad[0].layer).ok_or("unknown sprite layer")?;
            if quad.iter().any(|v: &Vertex| v.layer != quad[0].layer) {
                return Err("quad mixes sprite layers".into());
            }
            let vertices: Vec<Value> = quad
                .iter()
                .map(|v| {
                    json!([
                        v.pos[0],
                        v.pos[1],
                        v.pos[2],
                        v.uv[0],
                        v.uv[1],
                        argb(v.color),
                        light_coords(v.light)
                    ])
                })
                .collect();
            quads.push(json!({"layer": layer, "sprite": sprite, "vertices": vertices}));
        }
    }
    Ok(json!({"name": row["name"], "quads": quads}))
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 3 {
        return Err("usage: section_mesh_fixture RESOURCES INPUT.jsonl OUTPUT.jsonl".into());
    }
    let root = PathBuf::from(&args[0]);
    let assets = Assets::load(&root)?;
    let facts = BlockFacts::load(&root, assets.registry)?;
    let mut biomes = Biomes::load(&root)?;
    let visibility = models::load_visibility(&root, assets.registry)?;
    let tables = MeshTables::new(&assets, &facts, visibility)?;
    let names: BTreeMap<u32, &str> = assets
        .sprite_layers
        .iter()
        .map(|(name, layer)| (*layer, name.as_str()))
        .collect();
    let mut output = BufWriter::new(fs::File::create(&args[2])?);
    for line in BufReader::new(fs::File::open(&args[1])?).lines() {
        let row: Value = serde_json::from_str(&line?)?;
        let result = scene(&row, &assets, &tables, &mut biomes, &names)?;
        serde_json::to_writer(&mut output, &result)?;
        output.write_all(b"\n")?;
    }
    output.flush()?;
    Ok(())
}
