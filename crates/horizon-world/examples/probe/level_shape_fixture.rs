//! Rust side of `tools/test_level_shape_parity.py`: evaluates
//! `Behaviours::update_shape` and `can_survive` for generated neighbourhoods.
use horizon_core::{
    geometry::Direction,
    position::{BlockPos, ChunkPos},
    random::LegacyRandom,
};
use horizon_world::{
    level::{Behaviours, LevelAccess, Ticks, data::load_registry},
    registry::BlockStateId,
};
use serde_json::Value;
use std::{
    collections::{BTreeSet, HashMap},
    io::{BufRead, Write},
    path::Path,
};

struct Cells {
    cells: HashMap<BlockPos, BlockStateId>,
    air: BlockStateId,
    random: LegacyRandom,
    light: i32,
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
    fn sky_light(&self, _pos: BlockPos) -> i32 {
        self.light
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
            light: case["light"].as_i64().unwrap() as i32,
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
        let pos = BlockPos::new(p[0], p[1], p[2]);
        let direction = Direction::ALL[case["dir"].as_u64().unwrap() as usize];
        let [dx, dy, dz] = direction.normal();
        let neighbour_pos = pos.offset(dx, dy, dz);
        let state = level.block_state(pos);
        let neighbour = level.block_state(neighbour_pos);
        let shape = behaviours.update_shape(
            &mut level,
            &mut ticks,
            state,
            pos,
            direction,
            neighbour_pos,
            neighbour,
        );
        let mut scheduled = BTreeSet::new();
        for x in -2..=2 {
            for z in -2..=2 {
                let chunk = ChunkPos::new(x, z);
                for tick in ticks
                    .blocks
                    .chunk(chunk)
                    .map(|c| c.queued())
                    .unwrap_or_default()
                {
                    let name = &behaviours
                        .table
                        .info(horizon_world::level::BlockId(tick.kind.0 as u16))
                        .name;
                    scheduled.insert(format!(
                        "{name}@{},{},{}+{}p{}",
                        tick.pos.x, tick.pos.y, tick.pos.z, tick.trigger_tick, tick.priority as i32
                    ));
                }
                for tick in ticks
                    .fluids
                    .chunk(chunk)
                    .map(|c| c.queued())
                    .unwrap_or_default()
                {
                    let name = horizon_world::fluid::FluidKind::ALL[tick.kind.0 as usize].name();
                    scheduled.insert(format!(
                        "{name}@{},{},{}+{}p{}",
                        tick.pos.x, tick.pos.y, tick.pos.z, tick.trigger_tick, tick.priority as i32
                    ));
                }
            }
        }
        let survive = behaviours.can_survive(&mut level, &mut ticks, pos, state);
        let scheduled: Vec<String> = scheduled.into_iter().collect();
        writeln!(
            out,
            "shape {} ticks {} survive {survive}",
            shape.0,
            scheduled.join(" ")
        )?;
    }
    Ok(())
}
