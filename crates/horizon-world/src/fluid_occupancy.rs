//! Native section-ordered liquid occupancy over resident registered block states.
use crate::{
    chunk::BlockSection,
    compression::invalid,
    fluid::FluidState,
    registry::{BlockRegistry, BlockStateId},
};
use horizon_core::{geometry::Aabb, position::BlockPos};
use std::{fmt, io};

pub struct FluidPresence<'r> {
    registry: &'r BlockRegistry,
    present: Vec<bool>,
}
impl<'r> FluidPresence<'r> {
    pub fn new(registry: &'r BlockRegistry) -> io::Result<Self> {
        let present = (0..registry.len())
            .map(|i| {
                FluidState::from_block(registry.state(BlockStateId(i as u32)).unwrap())
                    .map(|f| f != FluidState::EMPTY)
            })
            .collect::<io::Result<Vec<_>>>()?;
        Ok(Self { registry, present })
    }
    pub fn registry(&self) -> &'r BlockRegistry {
        self.registry
    }
    pub fn has_fluid(&self, state: BlockStateId) -> io::Result<bool> {
        self.present
            .get(state.0 as usize)
            .copied()
            .ok_or_else(|| invalid("fluid presence state outside registry"))
    }
}
pub trait FluidOccupancyWorld {
    type Error;
    type Chunk: Copy;
    fn minimum_section_y(&mut self) -> Result<i32, Self::Error>;
    fn maximum_section_y(&mut self) -> Result<i32, Self::Error>;
    /// Native getChunk, including calls when the requested Y interval is empty.
    fn chunk(&mut self, x: i32, z: i32) -> Result<Self::Chunk, Self::Error>;
    fn section(&mut self, chunk: Self::Chunk, y: i32) -> Result<&BlockSection<'_>, Self::Error>;
    fn minimum_y(&mut self) -> Result<i32, Self::Error>;
    fn maximum_y(&mut self) -> Result<i32, Self::Error>;
}
#[derive(Debug)]
pub enum FluidOccupancyError<E> {
    World(E),
    Data(io::Error),
    NonFiniteBounds,
    BudgetExceeded,
}
impl<E: fmt::Display> fmt::Display for FluidOccupancyError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::World(e) => e.fmt(f),
            Self::Data(e) => e.fmt(f),
            Self::NonFiniteBounds => f.write_str("nonfinite fluid query bounds"),
            Self::BudgetExceeded => f.write_str("fluid query budget exceeded"),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for FluidOccupancyError<E> {}
fn spend<E>(budget: &mut usize) -> Result<(), FluidOccupancyError<E>> {
    if *budget == 0 {
        return Err(FluidOccupancyError::BudgetExceeded);
    }
    *budget -= 1;
    Ok(())
}
/// Inclusive floored AABB endpoints, as in LevelReader.findBlocksIn. Budget
/// counts chunk requests, section requests and inspected cells. Exhaustion is an
/// explicit error, never an empty-world result. Dry palettes avoid cell scans.
pub fn contains_any_liquid<W: FluidOccupancyWorld>(
    world: &mut W,
    registry: &FluidPresence<'_>,
    bounds: Aabb,
    mut budget: usize,
) -> Result<bool, FluidOccupancyError<W::Error>> {
    use FluidOccupancyError::{Data, World};
    let a = bounds.min_position();
    let b = bounds.max_position();
    if [a.x, a.y, a.z, b.x, b.y, b.z]
        .iter()
        .any(|v| !v.is_finite())
    {
        return Err(FluidOccupancyError::NonFiniteBounds);
    }
    let a = BlockPos::new(a.x.floor() as i32, a.y.floor() as i32, a.z.floor() as i32);
    let b = BlockPos::new(b.x.floor() as i32, b.y.floor() as i32, b.z.floor() as i32);
    let low = BlockPos::new(a.x.min(b.x), a.y.min(b.y), a.z.min(b.z));
    let high = BlockPos::new(a.x.max(b.x), a.y.max(b.y), a.z.max(b.z));
    let min_y = world.minimum_section_y().map_err(World)?.max(low.y >> 4);
    let max_y = world.maximum_section_y().map_err(World)?.min(high.y >> 4);
    for x in (low.x >> 4)..=(high.x >> 4) {
        for z in (low.z >> 4)..=(high.z >> 4) {
            spend(&mut budget)?;
            let chunk = world.chunk(x, z).map_err(World)?;
            for y in min_y..=max_y {
                spend(&mut budget)?;
                let section = world.section(chunk, y).map_err(World)?;
                if !std::ptr::eq(section.registry(), registry.registry) {
                    return Err(Data(invalid("fluid query section registry mismatch")));
                }
                if !section.any_state(|id| registry.present[id.0 as usize]) {
                    continue;
                }
                let origin = BlockPos::new(x.wrapping_shl(4), y.wrapping_shl(4), z.wrapping_shl(4));
                let min = BlockPos::new(
                    origin.x.max(low.x) & 15,
                    origin.y.max(low.y) & 15,
                    origin.z.max(low.z) & 15,
                );
                let max = BlockPos::new(
                    origin.x.wrapping_add(15).min(high.x) & 15,
                    origin.y.wrapping_add(15).min(high.y) & 15,
                    origin.z.wrapping_add(15).min(high.z) & 15,
                );
                for sy in min.y..=max.y {
                    for sz in min.z..=max.z {
                        for sx in min.x..=max.x {
                            spend(&mut budget)?;
                            let id = section
                                .get((sy * 256 + sz * 16 + sx) as usize)
                                .map_err(Data)?;
                            if registry.present[id.0 as usize] {
                                return Ok(true);
                            }
                        }
                    }
                }
            }
        }
    }
    // Native scans outside world height only if the predicate accepts air. It
    // still evaluates both height guards; the liquid predicate rejects air.
    world.minimum_y().map_err(World)?;
    world.maximum_y().map_err(World)?;
    Ok(false)
}
