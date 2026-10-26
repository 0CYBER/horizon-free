//! Default living jump, gravity, flight-speed and air-drag calculations.
use crate::living_air::modified_friction;
use horizon_core::geometry::{Vec3, trig};

pub trait LivingMotionWorld {
    type Error;
    fn velocity(&self) -> Vec3;
    fn set_velocity(&mut self, velocity: Vec3) -> Result<(), Self::Error>;
    fn jump_strength(&mut self) -> Result<f64, Self::Error>;
    fn block_jump_factor(&mut self) -> Result<f32, Self::Error>;
    fn has_jump_boost(&mut self) -> Result<bool, Self::Error>;
    fn jump_boost_amplifier(&mut self) -> Result<i32, Self::Error>;
    fn sprinting(&mut self) -> Result<bool, Self::Error>;
    fn yaw(&mut self) -> Result<f32, Self::Error>;
    fn mark_needs_sync(&mut self);
    fn no_gravity(&mut self) -> Result<bool, Self::Error>;
    fn gravity_attribute(&mut self) -> Result<f64, Self::Error>;
    fn has_slow_falling(&mut self) -> Result<bool, Self::Error>;
    fn controlling_passenger_is_player(&mut self) -> Result<bool, Self::Error>;
    fn speed(&mut self) -> Result<f32, Self::Error>;
    fn omnidirectional_air_mover(&mut self) -> Result<bool, Self::Error>;
    fn air_drag_modifier(&mut self) -> Result<f64, Self::Error>;
}
pub fn jump_boost_power<W: LivingMotionWorld>(world: &mut W) -> Result<f32, W::Error> {
    Ok(if world.has_jump_boost()? {
        0.1f32 * (world.jump_boost_amplifier()? as f32 + 1.0)
    } else {
        0.0
    })
}
pub fn jump_power<W: LivingMotionWorld>(world: &mut W, scale: f32) -> Result<f32, W::Error> {
    let strength = world.jump_strength()? as f32 * scale;
    let strength = strength * world.block_jump_factor()?;
    Ok(strength + jump_boost_power(world)?)
}
/// Native addDeltaMovement ignores nonfinite additions, but does not reject
/// a nonfinite existing velocity or overflow in the resulting sum.
pub fn add_delta_movement<W: LivingMotionWorld>(
    world: &mut W,
    delta: Vec3,
) -> Result<(), W::Error> {
    if delta.x.is_finite() && delta.y.is_finite() && delta.z.is_finite() {
        let old = world.velocity();
        world.set_velocity(Vec3::new(old.x + delta.x, old.y + delta.y, old.z + delta.z))?;
    }
    Ok(())
}
pub fn jump_from_ground<W: LivingMotionWorld>(world: &mut W) -> Result<(), W::Error> {
    let power = jump_power(world, 1.0)?;
    if power <= 1.0e-5f32 {
        return Ok(());
    }
    let mut velocity = world.velocity();
    velocity.y = if power.is_nan() || velocity.y.is_nan() {
        f64::NAN
    } else {
        f64::from(power).max(velocity.y)
    };
    world.set_velocity(velocity)?;
    if world.sprinting()? {
        let angle = f64::from(world.yaw()? * trig::DEG_TO_RAD);
        add_delta_movement(
            world,
            Vec3::new(
                f64::from(-trig::sin(angle)) * 0.2,
                0.0,
                f64::from(trig::cos(angle)) * 0.2,
            ),
        )?;
    }
    world.mark_needs_sync();
    Ok(())
}
pub fn gravity<W: LivingMotionWorld>(world: &mut W) -> Result<f64, W::Error> {
    if world.no_gravity()? {
        Ok(0.0)
    } else {
        world.gravity_attribute()
    }
}
pub fn effective_gravity<W: LivingMotionWorld>(world: &mut W) -> Result<f64, W::Error> {
    if world.velocity().y <= 0.0 && world.has_slow_falling()? {
        let gravity = gravity(world)?;
        Ok(if gravity.is_nan() {
            gravity
        } else {
            gravity.min(0.01)
        })
    } else {
        gravity(world)
    }
}
pub fn apply_gravity<W: LivingMotionWorld>(world: &mut W) -> Result<(), W::Error> {
    let gravity = gravity(world)?;
    if gravity != 0.0 {
        let old = world.velocity();
        world.set_velocity(Vec3::new(old.x + 0.0, old.y - gravity, old.z + 0.0))?;
    }
    Ok(())
}
pub fn swim_vertical<W: LivingMotionWorld>(world: &mut W, up: bool) -> Result<(), W::Error> {
    let old = world.velocity();
    let amount = f64::from(0.04f32);
    world.set_velocity(Vec3::new(
        old.x + 0.0,
        old.y + if up { amount } else { -amount },
        old.z + 0.0,
    ))
}
pub fn default_flying_speed<W: LivingMotionWorld>(world: &mut W) -> Result<f32, W::Error> {
    if world.controlling_passenger_is_player()? {
        Ok(world.speed()? * 0.1f32)
    } else {
        Ok(0.02)
    }
}
pub fn air_drag<W: LivingMotionWorld>(world: &mut W) -> Result<f32, W::Error> {
    let drag = if world.omnidirectional_air_mover()? {
        0.91
    } else {
        0.98
    };
    Ok(modified_friction(drag, world.air_drag_modifier()? as f32))
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_core::geometry::{Vec3, trig};

    struct Jumper {
        velocity: Vec3,
        sprinting: bool,
        yaw: f32,
        synced: bool,
    }
    impl LivingMotionWorld for Jumper {
        type Error = ();
        fn velocity(&self) -> Vec3 {
            self.velocity
        }
        fn set_velocity(&mut self, velocity: Vec3) -> Result<(), ()> {
            self.velocity = velocity;
            Ok(())
        }
        fn jump_strength(&mut self) -> Result<f64, ()> {
            Ok(0.42)
        }
        fn block_jump_factor(&mut self) -> Result<f32, ()> {
            Ok(1.0)
        }
        fn has_jump_boost(&mut self) -> Result<bool, ()> {
            Ok(false)
        }
        fn jump_boost_amplifier(&mut self) -> Result<i32, ()> {
            Ok(0)
        }
        fn sprinting(&mut self) -> Result<bool, ()> {
            Ok(self.sprinting)
        }
        fn yaw(&mut self) -> Result<f32, ()> {
            Ok(self.yaw)
        }
        fn mark_needs_sync(&mut self) {
            self.synced = true;
        }
        fn no_gravity(&mut self) -> Result<bool, ()> {
            Ok(false)
        }
        fn gravity_attribute(&mut self) -> Result<f64, ()> {
            Ok(0.08)
        }
        fn has_slow_falling(&mut self) -> Result<bool, ()> {
            Ok(false)
        }
        fn controlling_passenger_is_player(&mut self) -> Result<bool, ()> {
            Ok(false)
        }
        fn speed(&mut self) -> Result<f32, ()> {
            Ok(0.1)
        }
        fn omnidirectional_air_mover(&mut self) -> Result<bool, ()> {
            Ok(false)
        }
        fn air_drag_modifier(&mut self) -> Result<f64, ()> {
            Ok(0.0)
        }
    }

    fn impulse(yaw: f32) -> (f64, f64) {
        let angle = f64::from(yaw * trig::DEG_TO_RAD);
        (
            f64::from(-trig::sin(angle)) * 0.2,
            f64::from(trig::cos(angle)) * 0.2,
        )
    }

    /// LivingEntity.jumpFromGround: a sprint adds ~0.2 along look; a walk does not.
    #[test]
    fn sprint_jump_adds_look_direction_impulse() {
        for yaw in [0.0, 45.0, 90.0, 180.0, 270.0, 300.0] {
            let before = Vec3::new(0.13, -0.08, -0.05);
            let mut sprinting = Jumper {
                velocity: before,
                sprinting: true,
                yaw,
                synced: false,
            };
            jump_from_ground(&mut sprinting).unwrap();
            let (dx, dz) = impulse(yaw);
            let horizontal = ((dx * dx) + (dz * dz)).sqrt();
            assert!(
                (horizontal - 0.2).abs() < 1.0e-4,
                "yaw {yaw}: impulse length {horizontal}"
            );
            assert!(
                (sprinting.velocity.x - (before.x + dx)).abs() < 1.0e-9,
                "yaw {yaw}: x {:?} expected {}",
                sprinting.velocity,
                before.x + dx
            );
            assert!((sprinting.velocity.z - (before.z + dz)).abs() < 1.0e-9);
            // jump_power narrows the 0.42 strength through float, as vanilla does.
            let jump_y = f64::from(0.42_f32);
            assert!((sprinting.velocity.y - jump_y).abs() < 1.0e-9);
            assert!(sprinting.synced);

            let mut walking = Jumper {
                velocity: before,
                sprinting: false,
                yaw,
                synced: false,
            };
            jump_from_ground(&mut walking).unwrap();
            assert_eq!(walking.velocity.x, before.x);
            assert_eq!(walking.velocity.z, before.z);
            assert!((walking.velocity.y - jump_y).abs() < 1.0e-9);
        }
    }
}
