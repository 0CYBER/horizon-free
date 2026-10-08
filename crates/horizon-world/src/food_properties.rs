//! Food component listener: consumption sounds, stored hunger and burp ordering.
use crate::food::FoodData;
use horizon_core::{geometry::Vec3, random::RandomSource};
#[derive(Clone, Copy, Debug)]
pub struct FoodProperties {
    pub nutrition: i32,
    pub saturation: f32,
    pub can_always_eat: bool,
}
impl PartialEq for FoodProperties {
    fn eq(&self, other: &Self) -> bool {
        let bits = |f: f32| if f.is_nan() { 0x7fc00000 } else { f.to_bits() };
        self.nutrition == other.nutrition
            && bits(self.saturation) == bits(other.saturation)
            && self.can_always_eat == other.can_always_eat
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SoundCategory {
    Neutral,
    Players,
}
pub struct ConsumptionSound<L, S> {
    pub level: L,
    pub position: Vec3,
    pub sound: S,
    pub category: SoundCategory,
    pub volume: f32,
    pub pitch: f32,
}
pub trait FoodConsumptionWorld {
    type Error;
    type Level: Copy;
    type Sound: Clone;
    type Random: Copy;
    type Source: RandomSource;
    fn get_random(&mut self) -> Result<Self::Random, Self::Error>;
    fn random_source(&mut self, random: Self::Random) -> &mut Self::Source;
    /// Entity coordinate getters read its position field directly.
    fn position(&self) -> Vec3;
    fn is_player(&self) -> bool;
    /// The virtual player getter may change which stored FoodData is returned.
    fn food_data(&mut self) -> Result<&mut FoodData, Self::Error>;
    /// Native level sound call excludes no source entity (source argument null).
    fn play_sound(
        &mut self,
        sound: ConsumptionSound<Self::Level, Self::Sound>,
    ) -> Result<(), Self::Error>;
}
impl FoodProperties {
    pub fn on_consume<W: FoodConsumptionWorld>(
        &self,
        world: &mut W,
        level: W::Level,
        consume_sound: W::Sound,
        burp_sound: W::Sound,
    ) -> Result<(), W::Error> {
        let random = world.get_random()?;
        let position = world.position();
        let pitch = world.random_source(random).triangle_float(1.0, 0.4);
        world.play_sound(ConsumptionSound {
            level,
            position,
            sound: consume_sound,
            category: SoundCategory::Neutral,
            volume: 1.0,
            pitch,
        })?;
        if world.is_player() {
            world
                .food_data()?
                .eat_properties(self.nutrition, self.saturation);
            let position = world.position();
            let pitch = 0.9f32 + world.random_source(random).next_float() * (1.0f32 - 0.9f32);
            world.play_sound(ConsumptionSound {
                level,
                position,
                sound: burp_sound,
                category: SoundCategory::Players,
                volume: 0.5,
                pitch,
            })?;
        }
        Ok(())
    }
}
