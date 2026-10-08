//! Concrete non-teleport consume effects. Actor status storage and sound
//! delivery are supplied by the world; effect instance copies are native copies.
use crate::status_effect::EffectInstance;
use horizon_core::{position::BlockPos, random::RandomSource};

#[derive(Clone, Debug)]
pub enum ConsumeEffect<H, S, Set = Vec<H>> {
    ApplyStatus {
        effects: Vec<EffectInstance<H>>,
        probability: f32,
    },
    RemoveStatus(Set),
    ClearStatus,
    PlaySound(S),
}
impl<H: PartialEq, S: PartialEq, Set: PartialEq> PartialEq for ConsumeEffect<H, S, Set> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (
                Self::ApplyStatus {
                    effects: a,
                    probability: p,
                },
                Self::ApplyStatus {
                    effects: b,
                    probability: q,
                },
            ) => {
                let bits = |v: f32| if v.is_nan() { 0x7fc00000 } else { v.to_bits() };
                a == b && bits(*p) == bits(*q)
            }
            (Self::RemoveStatus(a), Self::RemoveStatus(b)) => a == b,
            (Self::ClearStatus, Self::ClearStatus) => true,
            (Self::PlaySound(a), Self::PlaySound(b)) => a == b,
            _ => false,
        }
    }
}
pub trait EffectWorld<H, S, Set> {
    type Error;
    type Level: Copy;
    type Random: Copy;
    type Source: RandomSource;
    type SoundCategory;
    fn get_random(&mut self) -> Result<Self::Random, Self::Error>;
    fn random_source(&mut self, random: Self::Random) -> &mut Self::Source;
    fn add_effect(&mut self, effect: EffectInstance<H>) -> Result<bool, Self::Error>;
    /// Resolve direct or named holder sets when execution begins. Native tag
    /// rebinding replaces the list; the current iteration retains its list.
    fn effect_set(&self, set: &Set) -> Vec<H>;
    fn remove_effect(&mut self, effect: H) -> Result<bool, Self::Error>;
    fn remove_all_effects(&mut self) -> Result<bool, Self::Error>;
    fn block_position(&mut self) -> Result<BlockPos, Self::Error>;
    fn sound_category(&mut self) -> Result<Self::SoundCategory, Self::Error>;
    /// Excluded entity is null; volume and pitch are both one.
    fn play_sound(
        &mut self,
        level: Self::Level,
        position: BlockPos,
        sound: S,
        category: Self::SoundCategory,
    ) -> Result<(), Self::Error>;
}
impl<H: Clone, S: Clone, Set> ConsumeEffect<H, S, Set> {
    pub fn apply<W: EffectWorld<H, S, Set>>(
        &self,
        world: &mut W,
        level: W::Level,
    ) -> Result<bool, W::Error> {
        match self {
            Self::ApplyStatus {
                effects,
                probability,
            } => {
                let random = world.get_random()?;
                if world.random_source(random).next_float() >= *probability {
                    return Ok(false);
                }
                let mut changed = false;
                for effect in effects {
                    if world.add_effect(effect.copy_instance())? {
                        changed = true;
                    }
                }
                Ok(changed)
            }
            Self::RemoveStatus(effects) => {
                let effects = world.effect_set(effects);
                let mut changed = false;
                for effect in effects {
                    if world.remove_effect(effect)? {
                        changed = true;
                    }
                }
                Ok(changed)
            }
            Self::ClearStatus => world.remove_all_effects(),
            Self::PlaySound(sound) => {
                let position = world.block_position()?;
                let sound = sound.clone();
                let category = world.sound_category()?;
                world.play_sound(level, position, sound, category)?;
                Ok(true)
            }
        }
    }
}
