//! Living effect particle, invisibility and glowing synchronization.
use crate::{active_effects::ActiveEffects, effect_particles::Particle, java_hash_map::JavaKey};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Presence {
    Invisibility,
    Glowing,
}

pub trait EffectSyncWorld<H: JavaKey + Clone> {
    type Error;
    type Data: Copy;
    type Level: Copy;
    /// Direct entityData field access, retaining the selected object.
    fn data(&self) -> Self::Data;
    fn store_particles(
        &mut self,
        effects: &mut ActiveEffects<H>,
        data: Self::Data,
        particles: Arc<[Particle]>,
    ) -> Result<(), Self::Error>;
    fn store_ambient(
        &mut self,
        effects: &mut ActiveEffects<H>,
        data: Self::Data,
        ambient: bool,
    ) -> Result<(), Self::Error>;
    /// Actual registered factories are pure and cannot mutate the active map.
    fn particle(&self, effect: &H, ambient: bool) -> Result<Particle, Self::Error>;
    fn has_effect(
        &mut self,
        effects: &mut ActiveEffects<H>,
        kind: Presence,
    ) -> Result<bool, Self::Error>;
    fn level(&mut self, effects: &mut ActiveEffects<H>) -> Result<Self::Level, Self::Error>;
    fn is_client(
        &mut self,
        effects: &mut ActiveEffects<H>,
        level: Self::Level,
    ) -> Result<bool, Self::Error>;
    fn glowing_tag(&self) -> bool;
    fn write_glowing_tag(&mut self, value: bool);
    fn read_flags(&self, data: Self::Data) -> Result<u8, Self::Error>;
    fn write_flags(
        &mut self,
        effects: &mut ActiveEffects<H>,
        data: Self::Data,
        flags: u8,
    ) -> Result<(), Self::Error>;
    fn get_flag(&mut self, _: &mut ActiveEffects<H>, bit: i32) -> Result<bool, Self::Error>
    where
        Self: Sized,
    {
        shared_flag(self, bit)
    }
    fn set_flag(
        &mut self,
        effects: &mut ActiveEffects<H>,
        bit: i32,
        value: bool,
    ) -> Result<(), Self::Error>
    where
        Self: Sized,
    {
        set_shared_flag(effects, self, bit, value)
    }
    fn set_invisible(
        &mut self,
        effects: &mut ActiveEffects<H>,
        value: bool,
    ) -> Result<(), Self::Error>
    where
        Self: Sized,
    {
        self.set_flag(effects, 5, value)
    }
    fn currently_glowing(&mut self, effects: &mut ActiveEffects<H>) -> Result<bool, Self::Error>
    where
        Self: Sized,
    {
        currently_glowing(effects, self)
    }
    fn remove_particles(&mut self, effects: &mut ActiveEffects<H>) -> Result<(), Self::Error>
    where
        Self: Sized,
    {
        remove_particles(effects, self)
    }
    fn update_invisibility(&mut self, effects: &mut ActiveEffects<H>) -> Result<(), Self::Error>
    where
        Self: Sized,
    {
        update_invisibility(effects, self)
    }
}
pub fn all_ambient<H: JavaKey + Clone>(effects: &ActiveEffects<H>) -> bool {
    effects.entries.entries().all(|(_, e)| {
        let e = e.read();
        !e.details.visible || e.details.ambient
    })
}
pub fn synchronize_particles<H: JavaKey + Clone, W: EffectSyncWorld<H>>(
    effects: &mut ActiveEffects<H>,
    world: &mut W,
) -> Result<(), W::Error> {
    // Registered factories have no world/map access. Their native stream pass
    // therefore preserves this order without a structural mutation callback.
    let particles = effects
        .entries
        .entries()
        .filter_map(|(_, e)| {
            let e = e.read();
            e.details
                .visible
                .then(|| world.particle(&e.effect, e.details.ambient))
        })
        .collect::<Result<Arc<[_]>, _>>()?;
    let data = world.data();
    world.store_particles(effects, data, particles)?;
    let data = world.data();
    let ambient = all_ambient(effects);
    world.store_ambient(effects, data, ambient)
}
pub fn remove_particles<H: JavaKey + Clone, W: EffectSyncWorld<H>>(
    effects: &mut ActiveEffects<H>,
    world: &mut W,
) -> Result<(), W::Error> {
    let data = world.data();
    world.store_particles(effects, data, Arc::from([]))
}
pub fn update_invisibility<H: JavaKey + Clone, W: EffectSyncWorld<H>>(
    effects: &mut ActiveEffects<H>,
    world: &mut W,
) -> Result<(), W::Error> {
    if effects.entries.is_empty() {
        world.remove_particles(effects)?;
        return world.set_invisible(effects, false);
    }
    let invisible = world.has_effect(effects, Presence::Invisibility)?;
    world.set_invisible(effects, invisible)?;
    synchronize_particles(effects, world)
}
pub fn shared_flag<H: JavaKey + Clone, W: EffectSyncWorld<H>>(
    world: &W,
    bit: i32,
) -> Result<bool, W::Error> {
    let flags = i32::from(world.read_flags(world.data())? as i8);
    Ok(flags & 1i32.wrapping_shl(bit as u32) != 0)
}
pub fn set_shared_flag<H: JavaKey + Clone, W: EffectSyncWorld<H>>(
    effects: &mut ActiveEffects<H>,
    world: &mut W,
    bit: i32,
    value: bool,
) -> Result<(), W::Error> {
    let flags = i32::from(world.read_flags(world.data())? as i8);
    let mask = 1i32.wrapping_shl(bit as u32);
    let value = if value { flags | mask } else { flags & !mask } as u8;
    let data = world.data();
    world.write_flags(effects, data, value)
}
pub fn currently_glowing<H: JavaKey + Clone, W: EffectSyncWorld<H>>(
    effects: &mut ActiveEffects<H>,
    world: &mut W,
) -> Result<bool, W::Error> {
    let level = world.level(effects)?;
    if !world.is_client(effects, level)? && world.has_effect(effects, Presence::Glowing)? {
        return Ok(true);
    }
    // Entity's base body repeats the virtual level/client lookups.
    let level = world.level(effects)?;
    if world.is_client(effects, level)? {
        world.get_flag(effects, 6)
    } else {
        Ok(world.glowing_tag())
    }
}
pub fn set_glowing_tag<H: JavaKey + Clone, W: EffectSyncWorld<H>>(
    effects: &mut ActiveEffects<H>,
    world: &mut W,
    value: bool,
) -> Result<(), W::Error> {
    world.write_glowing_tag(value);
    let glowing = world.currently_glowing(effects)?;
    world.set_flag(effects, 6, glowing)
}
pub fn update_glowing<H: JavaKey + Clone, W: EffectSyncWorld<H>>(
    effects: &mut ActiveEffects<H>,
    world: &mut W,
) -> Result<(), W::Error> {
    let glowing = world.currently_glowing(effects)?;
    if world.get_flag(effects, 6)? != glowing {
        world.set_flag(effects, 6, glowing)?;
    }
    Ok(())
}
pub fn update_dirty<H: JavaKey + Clone, W: EffectSyncWorld<H>>(
    effects: &mut ActiveEffects<H>,
    world: &mut W,
) -> Result<(), W::Error> {
    if effects.dirty {
        world.update_invisibility(effects)?;
        update_glowing(effects, world)?;
        effects.dirty = false;
    }
    Ok(())
}
