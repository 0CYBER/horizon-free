//! Entity swimming state transitions and the player's flying-ability guard.
use horizon_core::position::BlockPos;
pub trait SwimmingWorld {
    type Error;
    fn is_swimming(&mut self) -> Result<bool, Self::Error>;
    fn is_sprinting(&mut self) -> Result<bool, Self::Error>;
    fn is_in_water(&mut self) -> Result<bool, Self::Error>;
    fn is_under_water(&mut self) -> Result<bool, Self::Error>;
    fn is_passenger(&mut self) -> Result<bool, Self::Error>;
    fn was_eye_in_water(&self) -> bool;
    /// Direct blockPosition field, captured after preceding virtual queries.
    fn block_position(&self) -> BlockPos;
    fn water_at(&mut self, position: BlockPos) -> Result<bool, Self::Error>;
    fn set_swimming(&mut self, value: bool) -> Result<(), Self::Error>;
}
pub trait PlayerSwimmingWorld: SwimmingWorld {
    /// Direct abilities.flying field; do not dispatch getAbilities here.
    fn flying_ability(&self) -> bool;
}
pub fn under_water<W: SwimmingWorld>(world: &mut W) -> Result<bool, W::Error> {
    Ok(world.was_eye_in_water() && world.is_in_water()?)
}
pub fn shallow_water<W: SwimmingWorld>(world: &mut W) -> Result<bool, W::Error> {
    Ok(world.is_in_water()? && !world.is_under_water()?)
}
pub fn update<W: SwimmingWorld>(world: &mut W) -> Result<(), W::Error> {
    let next = if world.is_swimming()? {
        world.is_sprinting()? && world.is_in_water()? && !world.is_passenger()?
    } else {
        world.is_sprinting()?
            && world.is_under_water()?
            && !world.is_passenger()?
            && world.water_at(world.block_position())?
    };
    world.set_swimming(next)
}
pub fn update_player<W: PlayerSwimmingWorld>(world: &mut W) -> Result<(), W::Error> {
    if world.flying_ability() {
        world.set_swimming(false)
    } else {
        update(world)
    }
}
pub fn fluid_jump_threshold(eye_height: f32) -> f64 {
    if f64::from(eye_height) < 0.4 {
        0.0
    } else {
        0.4
    }
}
