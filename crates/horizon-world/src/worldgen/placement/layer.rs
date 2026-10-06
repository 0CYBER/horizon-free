//! Layer scanning and its interleaved count-provider sampling.
use super::{LayerBlock, PlacementContext, PlacementError};
use crate::{heightmap::HeightmapType, worldgen::value_provider::IntProvider};
use horizon_core::random::RandomSource;

pub(super) fn modify<R: RandomSource, E>(
    count: &IntProvider,
    context: &mut impl PlacementContext,
    random: &mut R,
    position: [i32; 3],
    emit: &mut impl FnMut(&mut R, [i32; 3]) -> Result<(), E>,
    remaining: &mut usize,
) -> Result<(), PlacementError<E>> {
    let mut layer = 0_i32;
    loop {
        let mut found = false;
        let mut attempt = 0_i32;
        loop {
            take(remaining)?;
            // Native samples on every loop condition, including the final check.
            if attempt >= count.sample(random)? {
                break;
            }
            let x = position[0].wrapping_add(random.next_int_bound(16)?);
            let z = position[2].wrapping_add(random.next_int_bound(16)?);
            let y = context.height(HeightmapType::MotionBlocking, x, z)?;
            if let Some(y) = ground(context, [x, y, z], layer, remaining)? {
                emit(random, [x, y, z]).map_err(PlacementError::Output)?;
                found = true;
            }
            attempt = attempt.wrapping_add(1);
        }
        layer = layer.wrapping_add(1);
        if !found {
            return Ok(());
        }
    }
}
fn ground<E>(
    context: &mut impl PlacementContext,
    position: [i32; 3],
    layer: i32,
    remaining: &mut usize,
) -> Result<Option<i32>, PlacementError<E>> {
    let [x, mut y, z] = position;
    let mut above = context.layer_block(position)?;
    let mut found = 0_i32;
    while y >= context.min_y()?.wrapping_add(1) {
        take(remaining)?;
        let below = context.layer_block([x, y.wrapping_sub(1), z])?;
        if below == LayerBlock::Ground && above == LayerBlock::Empty {
            if found == layer {
                return Ok((y != i32::MAX).then_some(y));
            }
            found = found.wrapping_add(1);
        }
        above = below;
        y = y.wrapping_sub(1);
    }
    Ok(None)
}
fn take<E>(remaining: &mut usize) -> Result<(), PlacementError<E>> {
    if *remaining == 0 {
        return Err(PlacementError::WorkLimit);
    }
    *remaining -= 1;
    Ok(())
}
