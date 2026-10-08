//! Base Entity fall-distance accumulation and landing callback ordering.
use horizon_core::{geometry::Vec3, position::BlockPos};

pub trait FallDamageWorld {
    type Error;
    type State: Copy;
    fn is_in_water(&mut self) -> Result<bool, Self::Error>;
    fn fall_distance(&self) -> f64;
    fn set_fall_distance(&mut self, distance: f64);
    fn fall_on(
        &mut self,
        state: Self::State,
        position: BlockPos,
        distance: f64,
    ) -> Result<(), Self::Error>;
    fn position(&self) -> Vec3;
    fn supporting_block(&self) -> Option<BlockPos>;
    fn block_state(&mut self, position: BlockPos) -> Result<Self::State, Self::Error>;
    fn hit_ground(&mut self, position: Vec3, state: Self::State) -> Result<(), Self::Error>;
    fn reset_fall_distance(&mut self) -> Result<(), Self::Error>;
}
/// LivingEntity's fluid update, landing particles and other overrides wrap this
/// base method separately. Errors preserve prior field changes and callbacks.
pub fn check_fall_damage<W: FallDamageWorld>(
    world: &mut W,
    dy: f64,
    on_ground: bool,
    state: W::State,
    position: BlockPos,
) -> Result<(), W::Error> {
    if !world.is_in_water()? && dy < 0.0 {
        world.set_fall_distance(world.fall_distance() - f64::from(dy as f32));
    }
    if on_ground {
        if world.fall_distance() > 0.0 {
            world.fall_on(state, position, world.fall_distance())?;
            // Landing callbacks may move the entity or replace its support.
            let position = world.position();
            let state = if let Some(support) = world.supporting_block() {
                world.block_state(support)?
            } else {
                state
            };
            world.hit_ground(position, state)?;
        }
        world.reset_fall_distance()?;
    }
    Ok(())
}
