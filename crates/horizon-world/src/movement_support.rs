//! Support-dependent block positions and Entity movement factors.
use horizon_core::{geometry::Vec3, position::BlockPos};

pub const LEGACY_SUPPORT_OFFSET: f32 = 0.2;
pub const MOVEMENT_SUPPORT_OFFSET: f32 = 0.500001;
pub const ON_SUPPORT_OFFSET: f32 = 1.0e-5;

/// State handles must remain valid for the duration of a query. Tag predicates
/// are separate to preserve native short circuits and reloadable tag membership.
pub trait MovementBlockWorld {
    type Error;
    type State: Copy;
    fn block_state(&mut self, position: BlockPos) -> Result<Self::State, Self::Error>;
    fn is_fence(&mut self, state: Self::State) -> Result<bool, Self::Error>;
    fn is_wall(&mut self, state: Self::State) -> Result<bool, Self::Error>;
    fn is_fence_gate(&mut self, state: Self::State) -> Result<bool, Self::Error>;
    fn is_water(&mut self, state: Self::State) -> Result<bool, Self::Error>;
    fn is_bubble_column(&mut self, state: Self::State) -> Result<bool, Self::Error>;
    fn jump_factor(&mut self, state: Self::State) -> Result<f32, Self::Error>;
    fn speed_factor(&mut self, state: Self::State) -> Result<f32, Self::Error>;
}
#[derive(Clone, Copy, Debug)]
pub struct MovementLocation {
    pub position: Vec3,
    /// Entity.blockPosition's captured value; do not recompute it here.
    pub block_position: BlockPos,
    pub supporting_block: Option<BlockPos>,
}
/// Entity.getOnPos(float). The offset is a float widened before subtraction.
pub fn support_position<W: MovementBlockWorld>(
    world: &mut W,
    position: Vec3,
    supporting_block: Option<BlockPos>,
    offset: f32,
) -> Result<BlockPos, W::Error> {
    if let Some(support) = supporting_block {
        if offset > ON_SUPPORT_OFFSET {
            let block = world.block_state(support)?;
            if (f64::from(offset) <= 0.5 && world.is_fence(block)?)
                || world.is_wall(block)?
                || world.is_fence_gate(block)?
            {
                return Ok(support);
            }
            Ok(BlockPos::new(
                support.x,
                (position.y - f64::from(offset)).floor() as i32,
                support.z,
            ))
        } else {
            Ok(support)
        }
    } else {
        Ok(BlockPos::new(
            position.x.floor() as i32,
            (position.y - f64::from(offset)).floor() as i32,
            position.z.floor() as i32,
        ))
    }
}
fn below<W: MovementBlockWorld>(
    world: &mut W,
    location: MovementLocation,
) -> Result<W::State, W::Error> {
    let position = support_position(
        world,
        location.position,
        location.supporting_block,
        MOVEMENT_SUPPORT_OFFSET,
    )?;
    world.block_state(position)
}
pub fn block_jump_factor<W: MovementBlockWorld>(
    world: &mut W,
    location: MovementLocation,
) -> Result<f32, W::Error> {
    let current = world.block_state(location.block_position)?;
    let current = world.jump_factor(current)?;
    // Native always evaluates the lower block, even if current is not one.
    let lower = below(world, location)?;
    let lower = world.jump_factor(lower)?;
    Ok(if current == 1.0 { lower } else { current })
}
pub fn block_speed_factor<W: MovementBlockWorld>(
    world: &mut W,
    location: MovementLocation,
) -> Result<f32, W::Error> {
    let current = world.block_state(location.block_position)?;
    let factor = world.speed_factor(current)?;
    if world.is_water(current)? || world.is_bubble_column(current)? || factor != 1.0 {
        return Ok(factor);
    }
    let lower = below(world, location)?;
    world.speed_factor(lower)
}
