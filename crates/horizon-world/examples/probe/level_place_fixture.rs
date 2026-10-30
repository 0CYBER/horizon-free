//! Rust side of `tools/test_level_place_parity.py`: evaluates
//! `Behaviours::placement_state` for generated neighbourhoods.
use horizon_core::{
    geometry::{Direction, Vec3},
    position::{BlockPos, ChunkPos},
    random::LegacyRandom,
};
use horizon_world::{
    level::{Behaviours, LevelAccess, PlaceContext, Ticks, data::load_registry},
    registry::BlockStateId,
};
use serde_json::Value;
use std::{
    collections::HashMap,
    io::{BufRead, Write},
    path::Path,
};

struct Cells {
    cells: HashMap<BlockPos, BlockStateId>,
    air: BlockStateId,
    random: LegacyRandom,
    obstructed: bool,
}
impl LevelAccess for Cells {
    fn block_state(&self, pos: BlockPos) -> BlockStateId {
        self.cells.get(&pos).copied().unwrap_or(self.air)
    }
    fn set_raw(&mut self, _pos: BlockPos, _state: BlockStateId) -> Option<BlockStateId> {
        None
    }
    fn is_loaded(&self, _pos: BlockPos) -> bool {
        true
    }
    fn random(&mut self) -> &mut LegacyRandom {
        &mut self.random
    }
    fn game_time(&self) -> i64 {
        0
    }
    fn is_unobstructed(&self, _pos: BlockPos, _state: BlockStateId) -> bool {
        !self.obstructed
    }
}

pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = crate::args().skip(1);
    let resources = args.next().expect("resource directory");
    let cases = args.next().expect("case file");
    let output = args.next().expect("output file");
    let resources = Path::new(&resources);
    let registry = load_registry(resources)?;
    let behaviours = Behaviours::load(&registry, resources)?;
    let air = registry.default_state("minecraft:air").unwrap();
    let mut out = std::io::BufWriter::new(std::fs::File::create(output)?);
    for line in std::io::BufReader::new(std::fs::File::open(cases)?).lines() {
        let case: Value = serde_json::from_str(&line?)?;
        let mut cells = HashMap::new();
        for cell in case["cells"].as_array().unwrap() {
            let c: Vec<i64> = cell
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_i64().unwrap())
                .collect();
            cells.insert(
                BlockPos::new(c[0] as i32, c[1] as i32, c[2] as i32),
                BlockStateId(c[3] as u32),
            );
        }
        let mut level = Cells {
            cells,
            air,
            random: LegacyRandom::new(case["seed"].as_i64().unwrap()),
            obstructed: case["obstructed"].as_bool().unwrap_or(false),
        };
        let mut ticks = Ticks::new(0);
        for x in -2..=2 {
            for z in -2..=2 {
                ticks.add_empty_chunk(ChunkPos::new(x, z))?;
            }
        }
        let p: Vec<i32> = case["pos"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_i64().unwrap() as i32)
            .collect();
        let h: Vec<f64> = case["hit"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_f64().unwrap())
            .collect();
        let ctx = PlaceContext {
            clicked_pos: BlockPos::new(p[0], p[1], p[2]),
            face: Direction::ALL[case["face"].as_u64().unwrap() as usize],
            hit: Vec3::new(h[0], h[1], h[2]),
            inside: false,
            yaw: case["yaw"].as_f64().unwrap() as f32,
            pitch: case["pitch"].as_f64().unwrap() as f32,
            sneaking: case["sneak"].as_bool().unwrap(),
            item: case["item"].as_str().unwrap().to_owned(),
            can_use_game_master_blocks: false,
        };
        match behaviours.placement_state(&mut level, &mut ticks, &ctx) {
            Some((pos, state)) => writeln!(out, "place {},{},{} {}", pos.x, pos.y, pos.z, state.0)?,
            None => writeln!(out, "place none")?,
        }
    }
    Ok(())
}
