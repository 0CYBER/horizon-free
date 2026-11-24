//! LivingEntity water/lava travel and fluid escape/buoyancy velocity updates.
use crate::living_air::{LivingAirWorld, move_relative};
use crate::move_gate::MoveGate;
use horizon_core::geometry::Vec3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TravelFluidTag {
    Lava,
    EntityFloatable,
}

/// The tunable inputs to the existing water and lava travel steps.
pub struct FluidTravelValues {
    pub acceleration: f32,
    pub horizontal_drag: f64,
    pub vertical_drag: f64,
    pub falling_adjustment: bool,
    pub extra_gravity_factor: f64,
    pub climb_impulse: Option<f64>,
}

pub trait LivingFluidWorld: LivingAirWorld {
    fn in_water(&mut self) -> Result<bool, Self::Error>;
    fn sprinting(&mut self) -> Result<bool, Self::Error>;
    fn water_slow_down(&mut self) -> Result<f32, Self::Error>;
    fn water_movement_efficiency(&mut self) -> Result<f64, Self::Error>;
    fn dolphins_grace(&mut self) -> Result<bool, Self::Error>;
    fn fluid_height(&mut self, tag: TravelFluidTag) -> Result<f64, Self::Error>;
    fn fluid_jump_threshold(&mut self) -> Result<f64, Self::Error>;
    fn is_free(&mut self, offset: Vec3) -> Result<bool, Self::Error>;
    fn is_vehicle(&mut self) -> Result<bool, Self::Error>;
    fn can_float_while_ridden(&mut self) -> Result<bool, Self::Error>;
    fn fluid_travel_listened(&self) -> bool {
        false
    }
    /// Water passes `None` for depth; an editor that needs it may read its water immersion.
    /// Lava passes `Some(deep)`, the depth used to choose its defaults.
    fn edit_fluid_travel(&mut self, _lava_deep: Option<bool>, _values: &mut FluidTravelValues) {}
}

pub fn fluid_falling_adjusted_movement<W: LivingFluidWorld>(
    world: &mut W,
    gravity: f64,
    falling: bool,
    mut velocity: Vec3,
) -> Result<Vec3, W::Error> {
    if gravity != 0.0 && !world.sprinting()? {
        velocity.y = if falling
            && (velocity.y - 0.005).abs() >= 0.003
            && (velocity.y - gravity / 16.0).abs() < 0.003
        {
            -0.003
        } else {
            velocity.y - gravity / 16.0
        };
    }
    Ok(velocity)
}
pub fn jump_out_of_fluid<W: LivingFluidWorld>(world: &mut W, old_y: f64) -> Result<(), W::Error> {
    let mut velocity = world.velocity();
    if world.horizontal_collision() {
        let probe = Vec3::new(
            velocity.x,
            velocity.y + f64::from(0.6f32) - world.y() + old_y,
            velocity.z,
        );
        if world.is_free(probe)? {
            velocity.y = f64::from(0.3f32);
            world.set_velocity(velocity)?;
        }
    }
    Ok(())
}
pub fn travel_in_water<W: LivingFluidWorld>(
    world: &mut W,
    gate: &mut MoveGate,
    input: Vec3,
    gravity: f64,
    falling: bool,
    old_y: f64,
) -> Result<(), W::Error> {
    let mut slowdown = if world.sprinting()? {
        0.9f32
    } else {
        world.water_slow_down()?
    };
    let mut speed = 0.02f32;
    let mut efficiency = world.water_movement_efficiency()? as f32;
    if !world.on_ground()? {
        efficiency *= 0.5;
    }
    if efficiency > 0.0 {
        slowdown += (0.54600006f32 - slowdown) * efficiency;
        speed += (world.speed()? - speed) * efficiency;
    }
    if world.dolphins_grace()? {
        slowdown = 0.96;
    }
    let mut values = FluidTravelValues {
        acceleration: speed,
        horizontal_drag: f64::from(slowdown),
        vertical_drag: f64::from(0.8f32),
        falling_adjustment: true,
        extra_gravity_factor: 0.0,
        climb_impulse: Some(0.2),
    };
    if world.fluid_travel_listened() {
        world.edit_fluid_travel(None, &mut values);
    }
    move_relative(world, input, values.acceleration)?;
    world.move_self(gate, world.velocity())?;
    let velocity = dragged_velocity(world, &values)?;
    let velocity = if values.falling_adjustment {
        fluid_falling_adjusted_movement(world, gravity, falling, velocity)?
    } else {
        velocity
    };
    world.set_velocity(velocity)?;
    add_extra_gravity(world, gravity, values.extra_gravity_factor)?;
    jump_out_of_fluid(world, old_y)
}
pub fn travel_in_lava<W: LivingFluidWorld>(
    world: &mut W,
    gate: &mut MoveGate,
    input: Vec3,
    gravity: f64,
    falling: bool,
    old_y: f64,
) -> Result<(), W::Error> {
    let defaults = |deep| FluidTravelValues {
        acceleration: 0.02,
        horizontal_drag: 0.5,
        vertical_drag: if deep { 0.5 } else { f64::from(0.8f32) },
        falling_adjustment: !deep,
        extra_gravity_factor: 0.25,
        climb_impulse: None,
    };
    let mut edited = None;
    if world.fluid_travel_listened() {
        // The policy needs the tracked depth before acceleration. Without a
        // listener keep vanilla's depth read after the move.
        let shallow = world.fluid_height(TravelFluidTag::Lava)? <= world.fluid_jump_threshold()?;
        let deep = !shallow;
        let mut values = defaults(deep);
        world.edit_fluid_travel(Some(deep), &mut values);
        edited = Some(values);
    }
    move_relative(
        world,
        input,
        edited.as_ref().map_or(0.02, |values| values.acceleration),
    )?;
    world.move_self(gate, world.velocity())?;
    let values = match edited {
        Some(values) => values,
        None => {
            let shallow =
                world.fluid_height(TravelFluidTag::Lava)? <= world.fluid_jump_threshold()?;
            defaults(!shallow)
        }
    };
    let velocity = dragged_velocity(world, &values)?;
    world.set_velocity(velocity)?;
    if values.falling_adjustment {
        let velocity = fluid_falling_adjusted_movement(world, gravity, falling, world.velocity())?;
        world.set_velocity(velocity)?;
    }
    add_extra_gravity(world, gravity, values.extra_gravity_factor)?;
    jump_out_of_fluid(world, old_y)
}

fn dragged_velocity<W: LivingFluidWorld>(
    world: &mut W,
    values: &FluidTravelValues,
) -> Result<Vec3, W::Error> {
    let mut velocity = world.velocity();
    if let Some(impulse) = values.climb_impulse
        && world.horizontal_collision()
        && world.on_climbable()?
    {
        velocity.y = impulse;
    }
    Ok(Vec3::new(
        velocity.x * values.horizontal_drag,
        velocity.y * values.vertical_drag,
        velocity.z * values.horizontal_drag,
    ))
}

fn add_extra_gravity<W: LivingFluidWorld>(
    world: &mut W,
    gravity: f64,
    factor: f64,
) -> Result<(), W::Error> {
    if factor != 0.0 && gravity != 0.0 {
        let velocity = world.velocity();
        world.set_velocity(Vec3::new(
            velocity.x + 0.0,
            velocity.y + -gravity * factor,
            velocity.z + 0.0,
        ))?;
    }
    Ok(())
}

pub fn float_in_liquid_while_ridden<W: LivingFluidWorld>(
    world: &mut W,
    tag: TravelFluidTag,
) -> Result<(), W::Error> {
    if world.is_vehicle()? && world.can_float_while_ridden()? {
        let threshold = world.fluid_jump_threshold()?;
        if world.fluid_height(tag)? > threshold {
            let velocity = world.velocity();
            world.set_velocity(Vec3::new(
                velocity.x + 0.0,
                velocity.y + f64::from(0.04f32),
                velocity.z + 0.0,
            ))?;
        }
    }
    Ok(())
}
pub fn travel_in_fluid<W: LivingFluidWorld>(
    world: &mut W,
    gate: &mut MoveGate,
    input: Vec3,
) -> Result<(), W::Error> {
    let falling = world.velocity().y <= 0.0;
    let old_y = world.y();
    let gravity = world.effective_gravity()?;
    if world.in_water()? {
        travel_in_water(world, gate, input, gravity, falling, old_y)?;
    } else {
        travel_in_lava(world, gate, input, gravity, falling, old_y)?;
    }
    float_in_liquid_while_ridden(world, TravelFluidTag::EntityFloatable)
}
