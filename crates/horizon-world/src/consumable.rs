//! Consumable component timing and ordered particle/sound emission.
use horizon_core::random::RandomSource;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UseAnimation {
    None,
    Eat,
    Drink,
    Block,
    Bow,
    Trident,
    Crossbow,
    Spyglass,
    TootHorn,
    Brush,
    Bundle,
    Spear,
}

#[derive(Clone, Debug)]
pub struct Consumable<S, E = ()> {
    pub seconds: f32,
    pub animation: UseAnimation,
    pub sound: S,
    pub particles: bool,
    pub effects: Vec<E>,
}
impl<S: PartialEq, E: PartialEq> PartialEq for Consumable<S, E> {
    fn eq(&self, other: &Self) -> bool {
        let bits = |v: f32| if v.is_nan() { 0x7fc00000 } else { v.to_bits() };
        bits(self.seconds) == bits(other.seconds)
            && self.animation == other.animation
            && self.sound == other.sound
            && self.particles == other.particles
            && self.effects == other.effects
    }
}

pub trait EmissionWorld {
    type Error;
    type Stack: Clone;
    type Sound: Clone;
    type Random: Copy;
    type Source: RandomSource;
    /// This is the supplied random source, which may differ from the actor's
    /// direct field used inside spawn_item_particles.
    fn random_source(&mut self, random: Self::Random) -> &mut Self::Source;
    fn spawn_item_particles(&mut self, stack: Self::Stack, count: i32) -> Result<(), Self::Error>;
    /// Native class/interface membership, not a virtual actor call.
    fn overrides_consume_sound(&self) -> bool;
    fn consume_sound(&mut self, stack: Self::Stack) -> Result<Self::Sound, Self::Error>;
    fn play_sound(
        &mut self,
        sound: Self::Sound,
        volume: f32,
        pitch: f32,
    ) -> Result<(), Self::Error>;
}
impl<S, E> Consumable<S, E> {
    pub fn ticks(&self) -> i32 {
        (self.seconds * 20.0) as i32
    }
    pub fn should_emit(&self, remaining: i32) -> bool {
        let elapsed = self.ticks().wrapping_sub(remaining);
        let threshold = (self.ticks() as f32 * 0.21875) as i32;
        elapsed > threshold && remaining % 4 == 0
    }
}
impl<S: Clone, E> Consumable<S, E> {
    pub fn emit<W: EmissionWorld<Sound = S>>(
        &self,
        world: &mut W,
        random: W::Random,
        stack: W::Stack,
        count: i32,
    ) -> Result<(), W::Error> {
        let source = world.random_source(random);
        let eat_volume = if source.next_boolean() { 0.5 } else { 1.0 };
        let eat_pitch = source.triangle_float(1.0, 0.2);
        let drink_pitch = 0.9f32 + source.next_float() * (1.0f32 - 0.9f32);
        let (volume, pitch) = if self.animation == UseAnimation::Drink {
            (0.5, drink_pitch)
        } else {
            (eat_volume, eat_pitch)
        };
        if self.particles {
            world.spawn_item_particles(stack.clone(), count)?;
        }
        let sound = if world.overrides_consume_sound() {
            world.consume_sound(stack)?
        } else {
            self.sound.clone()
        };
        world.play_sound(sound, volume, pitch)
    }
}
