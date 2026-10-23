//! Random-tick sections companion for tools/test_random_tick_sections_parity.py:
//! the game's `RandomTickSections` choosing the sections of a chunk that
//! `Behaviours::tick_chunk_random` ticks, after the case's edits (see
//! VanillaRandomTickSectionsOracle for the format; the `#resources` line names
//! the prepared resources). The level the ticks run against reads as air
//! everywhere, so nothing ticks for real: its reads are the drawn positions.
use horizon_client::{
    blocks::BlockFacts,
    chunk::{Chunk, MIN_Y, Section},
    random_tick_sections::RandomTickSections,
};
use horizon_core::{
    position::{BlockPos, ChunkPos},
    random::LegacyRandom,
};
use horizon_world::{
    fluid::FluidKind,
    level::{GameRules, LevelAccess, LevelEvent, Ticks, data},
    registry::BlockStateId,
};
use std::{cell::RefCell, error::Error, fmt::Write, path::PathBuf, sync::Arc};

struct Air {
    reads: RefCell<Vec<BlockPos>>,
    random: LegacyRandom,
    speed: i32,
}
impl LevelAccess for Air {
    fn block_state(&self, pos: BlockPos) -> BlockStateId {
        self.reads.borrow_mut().push(pos);
        BlockStateId(0)
    }
    fn set_raw(&mut self, _: BlockPos, _: BlockStateId) -> Option<BlockStateId> {
        None
    }
    fn is_loaded(&self, _: BlockPos) -> bool {
        true
    }
    fn random(&mut self) -> &mut LegacyRandom {
        &mut self.random
    }
    fn game_time(&self) -> i64 {
        0
    }
    fn game_rules(&self) -> GameRules {
        GameRules {
            random_tick_speed: self.speed,
            ..Default::default()
        }
    }
    fn emit(&mut self, _: LevelEvent) {}
}

pub fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = crate::args().collect();
    let input = std::fs::read_to_string(&args[1])?;
    let root = input
        .lines()
        .find_map(|l| l.strip_prefix("#resources "))
        .map(PathBuf::from)
        .ok_or("no #resources line")?;
    let registry = data::load_registry(&root)?;
    let (facts, behaviours) = BlockFacts::load_pair(&root, &registry)?;
    let mut output = String::new();
    for line in input.lines().filter(|l| !l.starts_with('#')) {
        let mut words = line.split(' ');
        let mut ticks = Ticks::new(0);
        ticks.random_tick_value = words.next().ok_or("no randValue")?.parse()?;
        let mut level = Air {
            reads: RefCell::default(),
            random: LegacyRandom::new(0),
            speed: words.next().ok_or("no speed")?.parse()?,
        };
        let mut chunk = Arc::new(Chunk::empty(0, 0));
        let mut cache = RandomTickSections::default();
        for word in words {
            let op: Vec<&str> = word.split(':').collect();
            let int = |i: usize| -> Result<i32, Box<dyn Error>> { Ok(op[i].parse()?) };
            match op[0] {
                "s" => {
                    let (section, index) = (int(1)?, int(2)?);
                    Arc::make_mut(&mut chunk).set(
                        index & 15,
                        MIN_Y + section * 16 + (index >> 8),
                        index >> 4 & 15,
                        int(3)? as u32,
                    );
                }
                "f" => {
                    let (a, b, m) = (int(2)? as u32, int(3)? as u32, int(4)? as usize);
                    let values = std::array::from_fn(|i| if i % m == m - 1 { b } else { a });
                    Arc::make_mut(&mut Arc::make_mut(&mut chunk).sections[int(1)? as usize])
                        .blocks = Section::from_blocks(&values);
                }
                "t" => {
                    output.push('|');
                    level.reads.borrow_mut().clear();
                    behaviours.tick_chunk_random(
                        &mut level,
                        &mut ticks,
                        ChunkPos::new(0, 0),
                        |_, from| cache.get((0, 0), &chunk, &facts).from(from),
                    );
                    for pos in level.reads.borrow().iter() {
                        let state = chunk.block(pos.x, pos.y, pos.z);
                        let fluid = matches!(
                            behaviours.fluid(state).kind(),
                            FluidKind::Lava | FluidKind::FlowingLava
                        );
                        write!(
                            output,
                            " {},{},{},{}{}",
                            pos.x,
                            pos.y,
                            pos.z,
                            if behaviours.is_randomly_ticking(state) {
                                'B'
                            } else {
                                '-'
                            },
                            if fluid { 'F' } else { '-' },
                        )?;
                    }
                }
                other => return Err(format!("unknown op {other}").into()),
            }
        }
        writeln!(output, " | {}", ticks.random_tick_value)?;
    }
    std::fs::write(&args[2], output)?;
    Ok(())
}
