//! Living entity status storage and add/replace/remove lifecycle ordering.
use crate::{
    java_hash_map::{JavaHashMap, JavaKey},
    status_effect::EffectInstance,
};

pub struct ActiveEffects<H> {
    /// LivingEntity schedules effect synchronization from construction.
    pub dirty: bool,
    /// Native activeEffects is exposed by getActiveEffectsMap. Direct mutations
    /// bypass lifecycle notifications, but retain the map's iterator semantics.
    pub entries: JavaHashMap<H, EffectInstance<H>>,
}
impl<H: JavaKey> Default for ActiveEffects<H> {
    fn default() -> Self {
        Self {
            entries: JavaHashMap::default(),
            dirty: true,
        }
    }
}
pub trait EffectHooks<H: JavaKey + Clone> {
    type Error;
    type Source: Clone;
    type Level: Copy;
    fn can_be_affected(
        &mut self,
        storage: &mut ActiveEffects<H>,
        incoming: EffectInstance<H>,
    ) -> Result<bool, Self::Error>;
    fn on_added(
        &mut self,
        storage: &mut ActiveEffects<H>,
        incoming: EffectInstance<H>,
        source: Option<Self::Source>,
    ) -> Result<(), Self::Error>;
    fn on_updated(
        &mut self,
        storage: &mut ActiveEffects<H>,
        existing: EffectInstance<H>,
        reapply: bool,
        source: Option<Self::Source>,
    ) -> Result<(), Self::Error>;
    /// Concrete MobEffect.onEffectAdded dispatch, after the actor callback.
    fn effect_added(
        &mut self,
        storage: &mut ActiveEffects<H>,
        effect: H,
        amplifier: i32,
    ) -> Result<(), Self::Error>;
    fn effect_started(
        &mut self,
        storage: &mut ActiveEffects<H>,
        effect: H,
        amplifier: i32,
    ) -> Result<(), Self::Error>;
    fn level(&mut self, storage: &mut ActiveEffects<H>) -> Result<Self::Level, Self::Error>;
    fn is_client(
        &mut self,
        storage: &mut ActiveEffects<H>,
        level: Self::Level,
    ) -> Result<bool, Self::Error>;
    fn on_removed(
        &mut self,
        storage: &mut ActiveEffects<H>,
        effects: Vec<EffectInstance<H>>,
    ) -> Result<(), Self::Error>;
    fn get_effect(
        &mut self,
        storage: &mut ActiveEffects<H>,
        effect: H,
    ) -> Result<Option<EffectInstance<H>>, Self::Error> {
        Ok(storage.get(&effect))
    }
    /// Entity.isRemoved is a final field-based query.
    fn actor_removed(&self) -> bool;
}
impl<H: JavaKey + Clone> ActiveEffects<H> {
    pub fn get(&self, effect: &H) -> Option<EffectInstance<H>> {
        self.entries.get(effect).cloned()
    }
    pub fn has(&self, effect: &H) -> bool {
        self.entries.get(effect).is_some()
    }
    pub fn add<W: EffectHooks<H>>(
        &mut self,
        incoming: EffectInstance<H>,
        source: Option<W::Source>,
        world: &mut W,
    ) -> Result<bool, W::Error> {
        if !world.can_be_affected(self, incoming.clone())? {
            return Ok(false);
        }
        let key = incoming.read().effect.clone();
        let existing = self.get(&key);
        let mut changed = false;
        if let Some(existing) = existing {
            if existing.update(&incoming) {
                world.on_updated(self, existing, true, source)?;
                changed = true;
            }
        } else {
            self.entries.put(key, incoming.clone());
            world.on_added(self, incoming.clone(), source)?;
            changed = true;
            let data = incoming.read();
            let effect = data.effect.clone();
            let amplifier = data.details.amplifier;
            drop(data);
            world.effect_added(self, effect, amplifier)?;
        }
        let data = incoming.read();
        let effect = data.effect.clone();
        let amplifier = data.details.amplifier;
        drop(data);
        world.effect_started(self, effect, amplifier)?;
        Ok(changed)
    }
    pub fn force_add<W: EffectHooks<H>>(
        &mut self,
        incoming: EffectInstance<H>,
        source: Option<W::Source>,
        world: &mut W,
    ) -> Result<(), W::Error> {
        if !world.can_be_affected(self, incoming.clone())? {
            return Ok(());
        }
        let key = incoming.read().effect.clone();
        let previous = self.entries.put(key, incoming.clone());
        if let Some(previous) = previous {
            incoming.copy_blend_from(&previous);
            world.on_updated(self, incoming, true, source)?;
        } else {
            world.on_added(self, incoming, source)?;
        }
        Ok(())
    }
    pub fn remove_no_update(&mut self, effect: &H) -> Option<EffectInstance<H>> {
        self.entries.remove(effect)
    }
    pub fn remove<W: EffectHooks<H>>(
        &mut self,
        effect: &H,
        world: &mut W,
    ) -> Result<bool, W::Error> {
        if let Some(instance) = self.remove_no_update(effect) {
            world.on_removed(self, vec![instance])?;
            Ok(true)
        } else {
            Ok(false)
        }
    }
    pub fn remove_all<W: EffectHooks<H>>(&mut self, world: &mut W) -> Result<bool, W::Error> {
        let level = world.level(self)?;
        if world.is_client(self, level)? || self.entries.is_empty() {
            return Ok(false);
        }
        let previous = self.entries.copied();
        self.entries.clear();
        let values = previous.entries().map(|(_, v)| v.clone()).collect();
        world.on_removed(self, values)?;
        Ok(true)
    }
    pub fn blend_factor<W: EffectHooks<H>>(
        &mut self,
        effect: H,
        partial: f32,
        world: &mut W,
    ) -> Result<f32, W::Error> {
        let effect = world.get_effect(self, effect)?;
        Ok(effect.map_or(0.0, |e| e.blend_factor(partial, world.actor_removed())))
    }
}
