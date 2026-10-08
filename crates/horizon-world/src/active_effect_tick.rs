//! LivingEntity effect tick loop, including native iterator failure handling.
use crate::{
    active_effects::{ActiveEffects, EffectHooks},
    java_hash_map::{CursorError, JavaKey},
    status_effect::{BlendWorld, EffectInstance, TickWorld},
};
use horizon_core::random::RandomSource;

pub trait TickHooks<H: JavaKey + Clone>: EffectHooks<H> {
    type Rng: RandomSource;
    type Particle: Clone;
    /// Runtime ServerLevel class test, distinct from Level.isClientSide.
    fn is_server_level(&self, level: Self::Level) -> bool;
    fn is_concurrent_modification(&self, error: &Self::Error) -> bool;
    fn actor_tick_count(&self) -> i32;
    fn should_apply(
        &mut self,
        storage: &mut ActiveEffects<H>,
        effect: H,
        time: i32,
        amplifier: i32,
    ) -> Result<bool, Self::Error>;
    fn apply_tick(
        &mut self,
        storage: &mut ActiveEffects<H>,
        effect: H,
        level: Self::Level,
        amplifier: i32,
    ) -> Result<bool, Self::Error>;
    fn advance_ticks(
        &mut self,
        storage: &mut ActiveEffects<H>,
        effect: H,
    ) -> Result<i32, Self::Error>;
    fn duration_ticks(
        &mut self,
        storage: &mut ActiveEffects<H>,
        effect: H,
        blending_in: bool,
    ) -> Result<i32, Self::Error>;
    /// Synced-data reads are direct and retain the selected particle list.
    fn particles(&self) -> Vec<Self::Particle>;
    fn ambient(&self) -> bool;
    fn invisible(&mut self, storage: &mut ActiveEffects<H>) -> Result<bool, Self::Error>;
    /// Direct actor random field access, without calling getRandom.
    fn random_source(&mut self) -> &mut Self::Rng;
    fn random_x(&mut self, storage: &mut ActiveEffects<H>, scale: f64) -> Result<f64, Self::Error>;
    fn random_y(&mut self, storage: &mut ActiveEffects<H>) -> Result<f64, Self::Error>;
    fn random_z(&mut self, storage: &mut ActiveEffects<H>, scale: f64) -> Result<f64, Self::Error>;
    /// Native particle velocity is (1, 1, 1).
    fn add_particle(
        &mut self,
        storage: &mut ActiveEffects<H>,
        level: Self::Level,
        particle: Self::Particle,
        position: [f64; 3],
    ) -> Result<(), Self::Error>;
}

#[derive(Debug)]
pub enum TickError<E> {
    Iterator(CursorError),
    Callback(E),
}
struct InstanceTick<'a, H, W> {
    storage: &'a mut ActiveEffects<H>,
    world: &'a mut W,
}
impl<H: JavaKey + Clone, W: TickHooks<H>> TickWorld<H> for InstanceTick<'_, H, W> {
    type Error = W::Error;
    type Level = W::Level;
    fn actor_tick_count(&self) -> i32 {
        self.world.actor_tick_count()
    }
    fn should_apply(
        &mut self,
        effect: H,
        time: i32,
        amplifier: i32,
        _: &EffectInstance<H>,
    ) -> Result<bool, Self::Error> {
        self.world
            .should_apply(self.storage, effect, time, amplifier)
    }
    fn apply_tick(
        &mut self,
        effect: H,
        level: Self::Level,
        amplifier: i32,
        _: &EffectInstance<H>,
    ) -> Result<bool, Self::Error> {
        self.world
            .apply_tick(self.storage, effect, level, amplifier)
    }
    fn downgraded(&mut self, instance: &EffectInstance<H>) -> Result<(), Self::Error> {
        self.world
            .on_updated(self.storage, instance.clone(), true, None)
    }
}
impl<H: JavaKey + Clone, W: TickHooks<H>> BlendWorld<H> for InstanceTick<'_, H, W> {
    type Error = W::Error;
    fn advance_ticks(&mut self, effect: H, _: &EffectInstance<H>) -> Result<i32, Self::Error> {
        self.world.advance_ticks(self.storage, effect)
    }
    fn duration_ticks(
        &mut self,
        effect: H,
        blending_in: bool,
        _: &EffectInstance<H>,
    ) -> Result<i32, Self::Error> {
        self.world.duration_ticks(self.storage, effect, blending_in)
    }
}

impl<H: JavaKey + Clone> ActiveEffects<H> {
    pub fn tick<W: TickHooks<H>>(&mut self, world: &mut W) -> Result<(), TickError<W::Error>> {
        let level = world.level(self).map_err(TickError::Callback)?;
        if world.is_server_level(level) {
            // The catch begins after the level query and iterator creation.
            match self.tick_server(world, level) {
                Err(TickError::Iterator(CursorError::ConcurrentModification)) => Ok(()),
                Err(TickError::Callback(error)) if world.is_concurrent_modification(&error) => {
                    Ok(())
                }
                result => result,
            }
        } else {
            self.tick_client(world)
        }
    }
    fn tick_server<W: TickHooks<H>>(
        &mut self,
        world: &mut W,
        level: W::Level,
    ) -> Result<(), TickError<W::Error>> {
        let mut cursor = self.entries.cursor();
        while cursor.has_next() {
            let (key, _) = self
                .entries
                .next_entry(&mut cursor)
                .map_err(TickError::Iterator)?
                .unwrap();
            let instance = self.get(key).unwrap();
            let active = instance
                .tick_server(
                    &mut InstanceTick {
                        storage: self,
                        world,
                    },
                    level,
                )
                .map_err(TickError::Callback)?;
            if !active {
                self.entries
                    .remove_current(&mut cursor)
                    .map_err(TickError::Iterator)?;
                world
                    .on_removed(self, vec![instance])
                    .map_err(TickError::Callback)?;
            } else if instance.read().details.duration % 600 == 0 {
                world
                    .on_updated(self, instance, false, None)
                    .map_err(TickError::Callback)?;
            }
        }
        Ok(())
    }
    fn tick_client<W: TickHooks<H>>(&mut self, world: &mut W) -> Result<(), TickError<W::Error>> {
        let mut cursor = self.entries.cursor();
        while cursor.has_next() {
            let (_, instance) = self
                .entries
                .next_entry(&mut cursor)
                .map_err(TickError::Iterator)?
                .unwrap();
            let instance = instance.clone();
            instance
                .tick_client(&mut InstanceTick {
                    storage: self,
                    world,
                })
                .map_err(TickError::Callback)?;
        }
        let particles = world.particles();
        if particles.is_empty() {
            return Ok(());
        }
        let ambient = world.ambient();
        let invisible = world.invisible(self).map_err(TickError::Callback)?;
        let interval = (if invisible { 15 } else { 4 }) * (if ambient { 5 } else { 1 });
        if world.random_source().next_int_bound(interval).unwrap() != 0 {
            return Ok(());
        }
        let level = world.level(self).map_err(TickError::Callback)?;
        let index = world
            .random_source()
            .next_int_bound(particles.len() as i32)
            .unwrap() as usize;
        let particle = particles[index].clone();
        let x = world.random_x(self, 0.5).map_err(TickError::Callback)?;
        let y = world.random_y(self).map_err(TickError::Callback)?;
        let z = world.random_z(self, 0.5).map_err(TickError::Callback)?;
        world
            .add_particle(self, level, particle, [x, y, z])
            .map_err(TickError::Callback)
    }
}
