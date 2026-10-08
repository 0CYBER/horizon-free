//! Consume effects bound to an entity's shared active-effect storage.
use crate::{
    active_effects::{ActiveEffects, EffectHooks},
    consume_effect::EffectWorld,
    java_hash_map::JavaKey,
    status_effect::EffectInstance,
};
use horizon_core::{position::BlockPos, random::RandomSource};

/// Actor and level services used alongside concrete status storage. Callbacks
/// receive the live storage so reentrant mutations remain visible to the next
/// operation. Random handles identify streams independently of actor changes.
pub trait ConsumeHooks<H: JavaKey + Clone, S, Set>: EffectHooks<H> {
    type Random: Copy;
    type Rng: RandomSource;
    type SoundCategory;
    fn get_random(&mut self, storage: &mut ActiveEffects<H>) -> Result<Self::Random, Self::Error>;
    fn random_source(&mut self, random: Self::Random) -> &mut Self::Rng;
    fn effect_set(&self, set: &Set) -> Vec<H>;
    fn block_position(&mut self, storage: &mut ActiveEffects<H>) -> Result<BlockPos, Self::Error>;
    fn sound_category(
        &mut self,
        storage: &mut ActiveEffects<H>,
    ) -> Result<Self::SoundCategory, Self::Error>;
    /// The native call excludes no entity and uses unit volume and pitch.
    fn play_sound(
        &mut self,
        storage: &mut ActiveEffects<H>,
        level: Self::Level,
        position: BlockPos,
        sound: S,
        category: Self::SoundCategory,
    ) -> Result<(), Self::Error>;
}

pub struct StoredConsume<'a, H, W> {
    pub effects: &'a mut ActiveEffects<H>,
    pub world: &'a mut W,
}
impl<H: JavaKey + Clone, S, Set, W: ConsumeHooks<H, S, Set>> EffectWorld<H, S, Set>
    for StoredConsume<'_, H, W>
{
    type Error = W::Error;
    type Level = W::Level;
    type Random = W::Random;
    type Source = W::Rng;
    type SoundCategory = W::SoundCategory;

    fn get_random(&mut self) -> Result<Self::Random, Self::Error> {
        self.world.get_random(self.effects)
    }
    fn random_source(&mut self, random: Self::Random) -> &mut Self::Source {
        self.world.random_source(random)
    }
    fn add_effect(&mut self, effect: EffectInstance<H>) -> Result<bool, Self::Error> {
        self.effects.add(effect, None, self.world)
    }
    fn effect_set(&self, set: &Set) -> Vec<H> {
        self.world.effect_set(set)
    }
    fn remove_effect(&mut self, effect: H) -> Result<bool, Self::Error> {
        self.effects.remove(&effect, self.world)
    }
    fn remove_all_effects(&mut self) -> Result<bool, Self::Error> {
        self.effects.remove_all(self.world)
    }
    fn block_position(&mut self) -> Result<BlockPos, Self::Error> {
        self.world.block_position(self.effects)
    }
    fn sound_category(&mut self) -> Result<Self::SoundCategory, Self::Error> {
        self.world.sound_category(self.effects)
    }
    fn play_sound(
        &mut self,
        level: Self::Level,
        position: BlockPos,
        sound: S,
        category: Self::SoundCategory,
    ) -> Result<(), Self::Error> {
        self.world
            .play_sound(self.effects, level, position, sound, category)
    }
}
