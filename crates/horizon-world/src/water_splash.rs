//! Water-entry sound, particles and game event, with native random draw order.
use horizon_core::geometry::Vec3;
use std::fmt;
#[derive(Clone, Copy, Debug)]
pub enum SplashParticle {
    Bubble,
    Splash,
}
pub trait WaterSplashWorld {
    type Error;
    type Entity: Copy + PartialEq;
    type Sound: Copy;
    fn this_entity(&self) -> Self::Entity;
    fn controlling_passenger(&mut self) -> Result<Option<Self::Entity>, Self::Error>;
    fn velocity(&mut self, entity: Self::Entity) -> Result<Vec3, Self::Error>;
    fn splash_sound(&mut self, high_speed: bool) -> Result<Self::Sound, Self::Error>;
    fn random_float(&mut self) -> Result<f32, Self::Error>;
    fn random_double(&mut self) -> Result<f64, Self::Error>;
    fn play_sound(
        &mut self,
        sound: Self::Sound,
        volume: f32,
        pitch: f32,
    ) -> Result<(), Self::Error>;
    fn position(&self) -> Vec3;
    fn width(&self) -> f32;
    fn particle(
        &mut self,
        kind: SplashParticle,
        position: Vec3,
        velocity: Vec3,
    ) -> Result<(), Self::Error>;
    fn splash_event(&mut self) -> Result<(), Self::Error>;
}
#[derive(Debug)]
pub enum SplashError<E> {
    World(E),
    BudgetExceeded,
}
impl<E: fmt::Display> fmt::Display for SplashError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::World(e) => e.fmt(f),
            Self::BudgetExceeded => f.write_str("water splash particle budget exceeded"),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for SplashError<E> {}
pub fn water_splash<W: WaterSplashWorld>(
    world: &mut W,
    mut budget: usize,
) -> Result<(), SplashError<W::Error>> {
    let source = world
        .controlling_passenger()
        .map_err(SplashError::World)?
        .unwrap_or(world.this_entity());
    let scale = if source == world.this_entity() {
        0.2f32
    } else {
        0.9f32
    };
    let velocity = world.velocity(source).map_err(SplashError::World)?;
    let energy = velocity.x * velocity.x * f64::from(0.2f32)
        + velocity.y * velocity.y
        + velocity.z * velocity.z * f64::from(0.2f32);
    let volume = energy.sqrt() as f32 * scale;
    let volume = if volume.is_nan() {
        volume
    } else {
        1.0f32.min(volume)
    };
    let sound = world
        .splash_sound(volume.is_nan() || volume >= 0.25)
        .map_err(SplashError::World)?;
    let pitch = 1.0
        + (world.random_float().map_err(SplashError::World)?
            - world.random_float().map_err(SplashError::World)?)
            * 0.4;
    world
        .play_sound(sound, volume, pitch)
        .map_err(SplashError::World)?;
    let surface = world.position().y.floor() as i32 as f32;
    for kind in [SplashParticle::Bubble, SplashParticle::Splash] {
        let mut i = 0i32;
        while (i as f32) < 1.0 + world.width() * 20.0 {
            if budget == 0 {
                return Err(SplashError::BudgetExceeded);
            }
            budget -= 1;
            let dx = (world.random_double().map_err(SplashError::World)? * 2.0 - 1.0)
                * f64::from(world.width());
            let dz = (world.random_double().map_err(SplashError::World)? * 2.0 - 1.0)
                * f64::from(world.width());
            let position = Vec3::new(
                world.position().x + dx,
                f64::from(surface + 1.0),
                world.position().z + dz,
            );
            let particle_velocity = match kind {
                SplashParticle::Bubble => Vec3::new(
                    velocity.x,
                    velocity.y
                        - world.random_double().map_err(SplashError::World)? * f64::from(0.2f32),
                    velocity.z,
                ),
                SplashParticle::Splash => velocity,
            };
            world
                .particle(kind, position, particle_velocity)
                .map_err(SplashError::World)?;
            i = i.wrapping_add(1);
        }
    }
    world.splash_event().map_err(SplashError::World)
}
