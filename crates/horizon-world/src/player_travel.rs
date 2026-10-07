//! Player-specific swimming and ability-flight behavior around living travel.
use horizon_core::{geometry::Vec3, position::BlockPos};
pub trait PlayerTravelWorld {
    type Error;
    fn is_passenger(&mut self) -> Result<bool, Self::Error>;
    fn is_swimming(&mut self) -> Result<bool, Self::Error>;
    fn look_angle(&mut self) -> Result<Vec3, Self::Error>;
    fn jumping(&self) -> bool;
    fn position(&self) -> Vec3;
    fn fluid_is_empty(&mut self, position: BlockPos) -> Result<bool, Self::Error>;
    fn velocity(&self) -> Vec3;
    fn set_velocity(&mut self, velocity: Vec3) -> Result<(), Self::Error>;
    /// Virtual getAbilities(), as distinct from direct abilities-field reads.
    fn query_flying_ability(&mut self) -> Result<bool, Self::Error>;
    fn flying_ability(&self) -> bool;
    fn ability_flying_speed(&mut self) -> Result<f32, Self::Error>;
    fn sprinting(&mut self) -> Result<bool, Self::Error>;
    fn living_travel(&mut self, input: Vec3) -> Result<(), Self::Error>;
}
pub fn travel<W: PlayerTravelWorld>(world: &mut W, input: Vec3) -> Result<(), W::Error> {
    if world.is_passenger()? {
        return world.living_travel(input);
    }
    if world.is_swimming()? {
        let look_y = world.look_angle()?.y;
        let scale = if look_y < -0.2 { 0.085 } else { 0.06 };
        let apply = if look_y <= 0.0 || world.jumping() {
            true
        } else {
            let p = world.position();
            !world.fluid_is_empty(BlockPos::new(
                p.x.floor() as i32,
                (p.y + 1.0 - 0.1).floor() as i32,
                p.z.floor() as i32,
            ))?
        };
        if apply {
            let v = world.velocity();
            world.set_velocity(Vec3::new(
                v.x + 0.0,
                v.y + (look_y - v.y) * scale,
                v.z + 0.0,
            ))?;
        }
    }
    if world.query_flying_ability()? {
        let y = world.velocity().y;
        world.living_travel(input)?;
        let v = world.velocity();
        world.set_velocity(Vec3::new(v.x, y * 0.6, v.z))?;
    } else {
        world.living_travel(input)?;
    }
    Ok(())
}
pub fn flying_speed<W: PlayerTravelWorld>(world: &mut W) -> Result<f32, W::Error> {
    if world.flying_ability() && !world.is_passenger()? {
        let sprinting = world.sprinting()?;
        let speed = world.ability_flying_speed()?;
        Ok(if sprinting { speed * 2.0 } else { speed })
    } else {
        Ok(if world.sprinting()? {
            0.025999999f32
        } else {
            0.02
        })
    }
}
pub fn affected_by_fluids<W: PlayerTravelWorld>(world: &W) -> bool {
    !world.flying_ability()
}
