use horizon_core::{geometry::Vec3, position::BlockPos};
use horizon_world::movement_support::{
    MovementBlockWorld, MovementLocation, player_block_speed_factor,
};

/// Air everywhere but a layer of soul sand (speed factor 0.4) at y = -61.
struct SoulSandFloor;
impl MovementBlockWorld for SoulSandFloor {
    type Error = std::convert::Infallible;
    type State = bool;
    fn block_state(&mut self, p: BlockPos) -> Result<bool, Self::Error> {
        Ok(p.y == -61)
    }
    fn is_fence(&mut self, _: bool) -> Result<bool, Self::Error> {
        Ok(false)
    }
    fn is_wall(&mut self, _: bool) -> Result<bool, Self::Error> {
        Ok(false)
    }
    fn is_fence_gate(&mut self, _: bool) -> Result<bool, Self::Error> {
        Ok(false)
    }
    fn is_water(&mut self, _: bool) -> Result<bool, Self::Error> {
        Ok(false)
    }
    fn is_bubble_column(&mut self, _: bool) -> Result<bool, Self::Error> {
        Ok(false)
    }
    fn jump_factor(&mut self, _: bool) -> Result<f32, Self::Error> {
        Ok(1.0)
    }
    fn speed_factor(&mut self, soul_sand: bool) -> Result<f32, Self::Error> {
        Ok(if soul_sand { 0.4 } else { 1.0 })
    }
}

/// `Player.getBlockSpeedFactor`: `!abilities.flying && !isFallFlying() ?
/// super.getBlockSpeedFactor() : 1.0F`. A creative player flying just above
/// soul sand (y -59.8 over a block whose top is -60.125, #16176) or gliding
/// over it keeps all its speed; walking on it (movement_efficiency 0) it
/// keeps 0.4.
#[test]
fn a_flying_or_gliding_player_is_not_slowed_by_soul_sand() {
    let position = Vec3::new(0.5, -59.8, 0.5);
    let location = MovementLocation {
        position,
        block_position: BlockPos::new(0, -60, 0),
        supporting_block: None,
    };
    // `abilities.flying || isFallFlying()`: either one lifts the block's factor.
    let factor = |flying| player_block_speed_factor(&mut SoulSandFloor, location, flying, 0.0);
    assert_eq!(factor(false), Ok(0.4), "walking");
    assert_eq!(factor(true), Ok(1.0), "flying or gliding");
}
