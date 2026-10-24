//! Transport for ClientLevel shape-aware terrain-particle placement.
use horizon_client::{assets::Assets, blocks::BlockFacts, mesher::MeshTables, models, particles};
use horizon_core::random::{LegacyRandom, RandomSource};
use horizon_world::registry::BlockStateId;
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, BufWriter, Write},
    path::Path,
};
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = crate::args().collect();
    let root = Path::new(&args[1]);
    let assets = Assets::load(root)?;
    let render = assets.reload(root)?;
    let facts = BlockFacts::load(root, assets.registry)?;
    let tables = MeshTables::new(
        &assets,
        &render,
        &facts,
        models::load_visibility(root, assets.registry)?,
    )?;
    let mut output = BufWriter::new(fs::File::create(&args[3])?);
    for line in BufReader::new(fs::File::open(&args[2])?).lines() {
        let row: Value = serde_json::from_str(&line?)?;
        let state = BlockStateId(row["state"].as_u64().unwrap() as u32);
        let pos: [i32; 3] = serde_json::from_value(row["pos"].clone())?;
        let offset = if facts.get(state).outline_offset {
            tables.offset(state, pos[0], pos[2]).unwrap_or([0.0; 3])
        } else {
            [0.0; 3]
        };
        let boxes: Vec<[f64; 6]> = facts
            .get(state)
            .outline
            .iter()
            .map(|b| std::array::from_fn(|i| b[i] + offset[i % 3]))
            .collect();
        let mut hits = Vec::new();
        let mut next = Vec::new();
        for face in [
            [0, -1, 0],
            [0, 1, 0],
            [0, 0, -1],
            [0, 0, 1],
            [-1, 0, 0],
            [1, 0, 0],
        ] {
            let mut random = LegacyRandom::new(row["seed"].as_i64().unwrap());
            hits.push(particles::breaking_position(&mut random, pos, face, &boxes));
            next.push(random.next_long());
        }
        let mut destroy = Vec::new();
        particles::destroy_positions(pos, &boxes, |p, _| destroy.push(p));
        serde_json::to_writer(
            &mut output,
            &json!({"case":row["case"],"hits":hits,"next":next,"destroy":destroy}),
        )?;
        output.write_all(&[10])?;
    }
    output.flush()?;
    Ok(())
}
