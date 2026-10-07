//! LivingEntity walking, climbing and airborne velocity integration.
use horizon_core::{
    geometry::{Vec3, trig},
    position::BlockPos,
};

pub trait LivingAirWorld {
    type Error;
    fn movement_support_position(&mut self) -> Result<BlockPos, Self::Error>;
    fn on_ground(&mut self) -> Result<bool, Self::Error>;
    fn block_friction(&mut self, position: BlockPos) -> Result<f32, Self::Error>;
    fn friction_modifier(&mut self) -> Result<f64, Self::Error>;
    fn speed(&mut self) -> Result<f32, Self::Error>;
    /// Includes entity-type overrides (notably Player).
    fn flying_speed(&mut self) -> Result<f32, Self::Error>;
    fn yaw(&mut self) -> Result<f32, Self::Error>;
    fn velocity(&self) -> Vec3;
    fn set_velocity(&mut self, velocity: Vec3) -> Result<(), Self::Error>;
    fn on_climbable(&mut self) -> Result<bool, Self::Error>;
    fn reset_fall_distance(&mut self) -> Result<(), Self::Error>;
    fn in_scaffolding(&mut self) -> Result<bool, Self::Error>;
    fn suppressing_ladder_slide(&mut self) -> Result<bool, Self::Error>;
    fn is_player_class(&self) -> bool;
    /// Bind to Entity.move(SELF, movement), including collision and landing.
    fn move_self(&mut self, movement: Vec3) -> Result<(), Self::Error>;
    fn horizontal_collision(&self) -> bool;
    fn jumping(&self) -> bool;
    fn was_in_powder_snow(&self) -> bool;
    fn can_walk_powder_snow(&mut self) -> Result<bool, Self::Error>;
    fn levitation_amplifier(&mut self) -> Result<Option<i32>, Self::Error>;
    fn is_client_side(&mut self) -> Result<bool, Self::Error>;
    fn has_chunk_at(&mut self, position: BlockPos) -> Result<bool, Self::Error>;
    fn effective_gravity(&mut self) -> Result<f64, Self::Error>;
    fn y(&self) -> f64;
    fn minimum_y(&mut self) -> Result<i32, Self::Error>;
    fn discard_friction(&mut self) -> Result<bool, Self::Error>;
    fn air_drag_modifier(&mut self) -> Result<f64, Self::Error>;
    fn omnidirectional_air_mover(&mut self) -> Result<bool, Self::Error>;
}

pub fn modified_friction(friction: f32, modifier: f32) -> f32 {
    (1.0 - (1.0 - friction) * modifier).clamp(0.0, 1.0)
}
/// Entity.getInputVector: normalize only above unit length, then rotate using
/// the native float-angle sine table. Tiny input returns canonical positive zero.
pub fn input_vector(input: Vec3, speed: f32, yaw: f32) -> Vec3 {
    let length = input.length_squared();
    if length < 1.0e-7 {
        return Vec3::ZERO;
    }
    let input = if length > 1.0 {
        input.normalize()
    } else {
        input
    };
    let input = input.scale(f64::from(speed));
    let angle = f64::from(yaw * trig::DEG_TO_RAD);
    let sin = f64::from(trig::sin(angle));
    let cos = f64::from(trig::cos(angle));
    Vec3::new(
        input.x * cos - input.z * sin,
        input.y,
        input.z * cos + input.x * sin,
    )
}
pub fn move_relative<W: LivingAirWorld>(
    world: &mut W,
    input: Vec3,
    speed: f32,
) -> Result<(), W::Error> {
    let input = input_vector(input, speed, world.yaw()?);
    let old = world.velocity();
    world.set_velocity(Vec3::new(old.x + input.x, old.y + input.y, old.z + input.z))
}
pub fn handle_on_climbable<W: LivingAirWorld>(
    world: &mut W,
    mut velocity: Vec3,
) -> Result<Vec3, W::Error> {
    if world.on_climbable()? {
        world.reset_fall_distance()?;
        let limit = f64::from(0.15f32);
        velocity.x = velocity.x.clamp(-limit, limit);
        velocity.z = velocity.z.clamp(-limit, limit);
        // Math.max must propagate NaN.
        if !velocity.y.is_nan() {
            velocity.y = velocity.y.max(-limit);
        }
        if velocity.y < 0.0
            && !world.in_scaffolding()?
            && world.suppressing_ladder_slide()?
            && world.is_player_class()
        {
            velocity.y = 0.0;
        }
    }
    Ok(velocity)
}
pub fn friction_influenced_speed<W: LivingAirWorld>(
    world: &mut W,
    friction: f32,
) -> Result<f32, W::Error> {
    if world.on_ground()? {
        let speed = world.speed()?;
        if f64::from(friction) > 0.6 {
            Ok(speed * (0.21600002f32 / (friction * friction * friction)))
        } else {
            Ok(speed)
        }
    } else {
        world.flying_speed()
    }
}
pub fn relative_friction_movement<W: LivingAirWorld>(
    world: &mut W,
    input: Vec3,
    friction: f32,
) -> Result<Vec3, W::Error> {
    let speed = friction_influenced_speed(world, friction)?;
    move_relative(world, input, speed)?;
    let velocity = handle_on_climbable(world, world.velocity())?;
    world.set_velocity(velocity)?;
    world.move_self(world.velocity())?;
    let mut velocity = world.velocity();
    if (world.horizontal_collision() || world.jumping())
        && (world.on_climbable()?
            || (world.was_in_powder_snow() && world.can_walk_powder_snow()?))
    {
        velocity.y = 0.2;
    }
    Ok(velocity)
}
/// Ground friction is captured before movement; drag and gravity are queried
/// afterwards. This distinction preserves movement callbacks' field changes.
pub fn travel_in_air<W: LivingAirWorld>(world: &mut W, input: Vec3) -> Result<(), W::Error> {
    let support = world.movement_support_position()?;
    let friction = if world.on_ground()? {
        let friction = world.block_friction(support)?;
        modified_friction(friction, world.friction_modifier()? as f32)
    } else {
        1.0
    };
    let mut velocity = relative_friction_movement(world, input, friction)?;
    if let Some(amplifier) = world.levitation_amplifier()? {
        velocity.y += (0.05 * f64::from(amplifier.wrapping_add(1)) - velocity.y) * 0.2;
    } else if !world.is_client_side()? || world.has_chunk_at(support)? {
        velocity.y -= world.effective_gravity()?;
    } else {
        let y = world.y();
        velocity.y = if y > f64::from(world.minimum_y()?) {
            -0.1
        } else {
            0.0
        };
    }
    if !world.discard_friction()? {
        let modifier = world.air_drag_modifier()? as f32;
        let drag = modified_friction(0.91, modifier);
        let horizontal = friction * drag;
        let vertical = if world.omnidirectional_air_mover()? {
            drag
        } else {
            modified_friction(0.98, modifier)
        };
        velocity = Vec3::new(
            velocity.x * f64::from(horizontal),
            velocity.y * f64::from(vertical),
            velocity.z * f64::from(horizontal),
        );
    }
    world.set_velocity(velocity)
}
