//! LivingEntity's fluid refresh and landing effects around base fall handling.
use crate::fall_damage::{FallDamageWorld, check_fall_damage};
use horizon_core::{geometry::Vec3, position::BlockPos};

pub trait LivingLandingWorld: FallDamageWorld {
    fn update_fluid_interaction(&mut self) -> Result<bool, Self::Error>;
    /// Native level type identity, independent of its client-side flag.
    fn is_server_level(&self) -> bool;
    fn on_changed_block(&mut self, position: BlockPos) -> Result<(), Self::Error>;
    fn safe_fall_distance(&mut self) -> Result<f64, Self::Error>;
    fn state_is_air(&mut self, state: Self::State) -> Result<bool, Self::Error>;
    fn block_position(&mut self) -> Result<BlockPos, Self::Error>;
    /// Dispatch BLOCK particles with zero axis spread and the supplied speed.
    fn landing_particles(
        &mut self,
        state: Self::State,
        position: Vec3,
        count: i32,
        speed: f64,
    ) -> Result<(), Self::Error>;
    fn clear_last_climbable_position(&mut self);
}

/// Preserve callbacks' field mutations and partial state on failure. The
/// landing position and state are the original movement phase's arguments.
pub fn check_living_fall_damage<W: LivingLandingWorld>(
    world: &mut W,
    dy: f64,
    on_ground: bool,
    state: W::State,
    landing: BlockPos,
) -> Result<(), W::Error> {
    if !world.is_in_water()? {
        world.update_fluid_interaction()?;
    }
    if world.is_server_level() && on_ground && world.fall_distance() > 0.0 {
        world.on_changed_block(landing)?;
        let power = world.fall_distance() + 1.0e-6 - world.safe_fall_distance()?;
        let power = f64::from((power.floor() as i32).max(0));
        if power > 0.0 && !world.state_is_air(state)? {
            let mut position = world.position();
            let current = world.block_position()?;
            if landing.x != current.x || landing.z != current.z {
                let dx = position.x - f64::from(landing.x) - 0.5;
                let dz = position.z - f64::from(landing.z) - 0.5;
                // Math.max propagates NaN; Rust's max selects the other operand.
                let scale = if dx.is_nan() || dz.is_nan() {
                    f64::NAN
                } else {
                    dx.abs().max(dz.abs())
                };
                position.x = f64::from(landing.x) + 0.5 + dx / scale * 0.5;
                position.z = f64::from(landing.z) + 0.5 + dz / scale * 0.5;
            }
            let magnitude = (f64::from(0.2f32) + power / 15.0).min(2.5);
            world.landing_particles(
                state,
                position,
                (150.0 * magnitude) as i32,
                f64::from(0.15f32),
            )?;
        }
    }
    check_fall_damage(world, dy, on_ground, state, landing)?;
    if on_ground {
        world.clear_last_climbable_position();
    }
    Ok(())
}
