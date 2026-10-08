//! Eating particles retain one item template while reading live actor state.
use crate::{
    item_stack_refs::StackRef,
    item_template::{ItemTemplate, TemplateError},
};
use horizon_core::{
    geometry::{Vec3, trig::DEG_TO_RAD},
    random::RandomSource,
};

pub trait ItemParticleWorld<I, K, V> {
    type Error;
    type Level;
    type Source: RandomSource;
    /// The actor's direct random field, not its virtual getRandom result.
    fn random(&mut self) -> &mut Self::Source;
    fn pitch(&mut self) -> Result<f32, Self::Error>;
    fn yaw(&mut self) -> Result<f32, Self::Error>;
    fn position(&self) -> Vec3;
    fn eye_y(&mut self) -> Result<f64, Self::Error>;
    fn level(&mut self) -> Result<Self::Level, Self::Error>;
    fn add_particle(
        &mut self,
        level: Self::Level,
        item: &ItemTemplate<I, K, V>,
        position: Vec3,
        velocity: Vec3,
    ) -> Result<(), Self::Error>;
}

#[derive(Debug)]
pub enum ParticleError<E> {
    Template(TemplateError),
    World(E),
}

pub fn spawn<I: Copy + Eq, K: Clone + Eq, V: Clone + PartialEq, W: ItemParticleWorld<I, K, V>>(
    world: &mut W,
    stack: &StackRef<I, K, V>,
    air: I,
    count: i32,
) -> Result<(), ParticleError<W::Error>> {
    let template = {
        let stack = stack.read();
        if stack.is_empty() {
            return Ok(());
        }
        ItemTemplate::from_non_empty_stack(&stack, air).map_err(ParticleError::Template)?
    };
    for _ in 0..count {
        let mut velocity = Vec3::new(
            (f64::from(world.random().next_float()) - 0.5) * 0.1,
            f64::from(world.random().next_float()) * 0.1 + 0.1,
            0.0,
        );
        velocity = velocity.rotate_x(-world.pitch().map_err(ParticleError::World)? * DEG_TO_RAD);
        velocity = velocity.rotate_y(-world.yaw().map_err(ParticleError::World)? * DEG_TO_RAD);
        let y = f64::from(-world.random().next_float()) * 0.6 - 0.3;
        let mut position = Vec3::new((f64::from(world.random().next_float()) - 0.5) * 0.3, y, 0.6);
        position = position.rotate_x(-world.pitch().map_err(ParticleError::World)? * DEG_TO_RAD);
        position = position.rotate_y(-world.yaw().map_err(ParticleError::World)? * DEG_TO_RAD);
        let x = world.position().x;
        let eye_y = world.eye_y().map_err(ParticleError::World)?;
        let z = world.position().z;
        position = position.add_vector(Vec3::new(x, eye_y, z));
        let level = world.level().map_err(ParticleError::World)?;
        velocity.y += 0.05;
        world
            .add_particle(level, &template, position, velocity)
            .map_err(ParticleError::World)?;
    }
    Ok(())
}
