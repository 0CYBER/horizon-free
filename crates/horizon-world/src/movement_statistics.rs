//! ServerPlayer.checkMovementStatistics. Called for accepted movement, separately
//! from Entity.move/travel: the packet listener owns that boundary in vanilla.
use horizon_core::geometry::Vec3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MovementStat {
    Swim,
    WalkUnderWater,
    WalkOnWater,
    Climb,
    Sprint,
    Crouch,
    Walk,
    Aviate,
    Fly,
}
impl MovementStat {
    pub fn name(self) -> &'static str {
        match self {
            Self::Swim => "swim_one_cm",
            Self::WalkUnderWater => "walk_under_water_one_cm",
            Self::WalkOnWater => "walk_on_water_one_cm",
            Self::Climb => "climb_one_cm",
            Self::Sprint => "sprint_one_cm",
            Self::Crouch => "crouch_one_cm",
            Self::Walk => "walk_one_cm",
            Self::Aviate => "aviate_one_cm",
            Self::Fly => "fly_one_cm",
        }
    }
}
pub trait MovementStatisticsWorld {
    type Error;
    fn passenger(&mut self) -> Result<bool, Self::Error>;
    fn swimming(&mut self) -> Result<bool, Self::Error>;
    fn eyes_in_water(&mut self) -> Result<bool, Self::Error>;
    fn in_water(&mut self) -> Result<bool, Self::Error>;
    fn on_climbable(&mut self) -> Result<bool, Self::Error>;
    fn on_ground(&mut self) -> Result<bool, Self::Error>;
    fn sprinting(&mut self) -> Result<bool, Self::Error>;
    fn crouching(&mut self) -> Result<bool, Self::Error>;
    fn fall_flying(&mut self) -> Result<bool, Self::Error>;
    fn award(&mut self, stat: MovementStat, amount: i32) -> Result<(), Self::Error>;
    fn exhaust(&mut self, amount: f32) -> Result<(), Self::Error>;
}
fn round_float(value: f32) -> i32 {
    let bits = value.to_bits() as i32;
    let shift = 149 - ((bits >> 23) & 0xff);
    if (0..32).contains(&shift) {
        let mut significand = (bits & 0x7f_ffff) | 0x80_0000;
        if bits < 0 {
            significand = -significand;
        }
        ((significand >> shift) + 1) >> 1
    } else {
        value as i32
    }
}
fn round_double(value: f64) -> i64 {
    let bits = value.to_bits() as i64;
    let shift = 1074 - ((bits >> 52) & 0x7ff);
    if (0..64).contains(&shift) {
        let mut significand = (bits & 0xf_ffff_ffff_ffff) | 0x10_0000_0000_0000;
        if bits < 0 {
            significand = -significand;
        }
        ((significand >> shift) + 1) >> 1
    } else {
        value as i64
    }
}
fn centimeters(squared: f64) -> i32 {
    // Java narrows the square root BEFORE multiplying by 100f and rounding.
    round_float(squared.sqrt() as f32 * 100.0)
}
fn distance_with_exhaustion<W: MovementStatisticsWorld>(
    world: &mut W,
    stat: MovementStat,
    cm: i32,
    rate: f32,
) -> Result<(), W::Error> {
    if cm > 0 {
        world.award(stat, cm)?;
        world.exhaust(rate * cm as f32 * 0.01)?;
    }
    Ok(())
}
pub fn check<W: MovementStatisticsWorld>(world: &mut W, delta: Vec3) -> Result<(), W::Error> {
    if world.passenger()? || (delta.x == 0.0 && delta.y == 0.0 && delta.z == 0.0) {
        return Ok(());
    }
    let distance_3d = || centimeters(delta.x * delta.x + delta.y * delta.y + delta.z * delta.z);
    let horizontal = || centimeters(delta.x * delta.x + delta.z * delta.z);
    if world.swimming()? {
        distance_with_exhaustion(world, MovementStat::Swim, distance_3d(), 0.01)
    } else if world.eyes_in_water()? {
        distance_with_exhaustion(world, MovementStat::WalkUnderWater, distance_3d(), 0.01)
    } else if world.in_water()? {
        distance_with_exhaustion(world, MovementStat::WalkOnWater, horizontal(), 0.01)
    } else if world.on_climbable()? {
        if delta.y > 0.0 {
            // The native double round returns long; l2i then wraps, not saturates.
            world.award(MovementStat::Climb, round_double(delta.y * 100.0) as i32)?;
        }
        Ok(())
    } else if world.on_ground()? {
        let cm = horizontal();
        if cm > 0 {
            let (stat, rate) = if world.sprinting()? {
                (MovementStat::Sprint, 0.1)
            } else if world.crouching()? {
                (MovementStat::Crouch, 0.0)
            } else {
                (MovementStat::Walk, 0.0)
            };
            distance_with_exhaustion(world, stat, cm, rate)?;
        }
        Ok(())
    } else if world.fall_flying()? {
        world.award(MovementStat::Aviate, distance_3d())
    } else {
        let cm = horizontal();
        if cm > 25 {
            world.award(MovementStat::Fly, cm)?;
        }
        Ok(())
    }
}
