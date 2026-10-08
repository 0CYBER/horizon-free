//! Shared cave/canyon ellipsoid traversal and path reachability.
use super::{CarvingMask, CarvingMaskError};
use crate::{geometry::floor_i32, position::ChunkPos};

pub trait CarverOutput {
    type Error;
    fn min_y(&self) -> i32;
    fn max_y(&self) -> i32;
    fn carve(&mut self, x: i32, y: i32, z: i32) -> Result<(), Self::Error>;
}
impl CarverOutput for CarvingMask {
    type Error = CarvingMaskError;
    fn min_y(&self) -> i32 {
        CarvingMask::min_y(self)
    }
    fn max_y(&self) -> i32 {
        CarvingMask::max_y(self)
    }
    fn carve(&mut self, x: i32, y: i32, z: i32) -> Result<(), Self::Error> {
        CarvingMask::carve(self, x, y, z)
    }
}
#[derive(Debug, PartialEq, Eq)]
pub enum CarveError<E> {
    Output(E),
    IterationLimit,
}
impl<E: std::fmt::Display> std::fmt::Display for CarveError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Output(e) => e.fmt(f),
            Self::IterationLimit => f.write_str("carver traversal budget exhausted"),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for CarveError<E> {}

/// Tests a cave/canyon path against the target chunk's center. Thickness math
/// first rounds in float precision, while distances use doubles.
pub fn can_reach(chunk: ChunkPos, x: f64, z: f64, step: i32, end: i32, thickness: f32) -> bool {
    let dx = x - f64::from(chunk.x.wrapping_shl(4).wrapping_add(8));
    let dz = z - f64::from(chunk.z.wrapping_shl(4).wrapping_add(8));
    let remaining = f64::from(end.wrapping_sub(step));
    let radius = f64::from((thickness + 2.) + 16.);
    dx * dx + dz * dz - remaining * remaining <= radius * radius
}

/// Visit candidate cells in X/Z/descending-Y order. The lower Y limit is
/// exclusive; normalized vertical coordinates use Y-0.5. The caller supplies
/// the cave/canyon skip predicate, which receives each candidate's absolute Y.
/// Failure retains previous output writes. Work is capped at 2^24 candidates.
pub fn carve_ellipsoid<O: CarverOutput>(
    chunk: ChunkPos,
    center: [f64; 3],
    horizontal: f64,
    vertical: f64,
    output: &mut O,
    skip: impl FnMut([f64; 3], i32) -> bool,
) -> Result<(), CarveError<O::Error>> {
    carve_ellipsoid_with_budget(
        chunk,
        center,
        horizontal,
        vertical,
        output,
        skip,
        &mut (1 << 24),
    )
}

/// The same traversal with a shared candidate budget for a complete carver path.
/// Each tested cell consumes one unit, including cells rejected by the predicate.
pub fn carve_ellipsoid_with_budget<O: CarverOutput>(
    chunk: ChunkPos,
    [x, y, z]: [f64; 3],
    horizontal: f64,
    vertical: f64,
    output: &mut O,
    mut skip: impl FnMut([f64; 3], i32) -> bool,
    remaining: &mut usize,
) -> Result<(), CarveError<O::Error>> {
    let cx = chunk.x.wrapping_shl(4);
    let cz = chunk.z.wrapping_shl(4);
    let reach = 16. + horizontal * 2.;
    if (x - f64::from(cx.wrapping_add(8))).abs() > reach
        || (z - f64::from(cz.wrapping_add(8))).abs() > reach
    {
        return Ok(());
    }
    let min_x = floor_i32(x - horizontal)
        .wrapping_sub(cx)
        .wrapping_sub(1)
        .max(0);
    let max_x = floor_i32(x + horizontal).wrapping_sub(cx).min(15);
    let min_y = floor_i32(y - vertical).wrapping_sub(1).max(output.min_y());
    let max_y = floor_i32(y + vertical).wrapping_add(1).min(output.max_y());
    let min_z = floor_i32(z - horizontal)
        .wrapping_sub(cz)
        .wrapping_sub(1)
        .max(0);
    let max_z = floor_i32(z + horizontal).wrapping_sub(cz).min(15);
    for lx in min_x..=max_x {
        let nx = (f64::from(cx.wrapping_add(lx)) + 0.5 - x) / horizontal;
        for lz in min_z..=max_z {
            let nz = (f64::from(cz.wrapping_add(lz)) + 0.5 - z) / horizontal;
            if nx * nx + nz * nz >= 1. {
                continue;
            }
            let mut by = max_y;
            while by > min_y {
                if *remaining == 0 {
                    return Err(CarveError::IterationLimit);
                }
                *remaining -= 1;
                let ny = (f64::from(by) - 0.5 - y) / vertical;
                if !skip([nx, ny, nz], by) {
                    output.carve(lx, by, lz).map_err(CarveError::Output)?;
                }
                by -= 1;
            }
        }
    }
    Ok(())
}
