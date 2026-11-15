//! Code-defined particle providers, as a client mod registers them in `register_particle_providers`
//! (NeoForge's `RegisterParticleProvidersEvent` and `ParticleProvider`).
//!
//! A provider owns one of the mod's own particle types, named as `ParticleOptions::named` names it
//! (`mymod:spark`). The particle is built as a declarative one is (the sprite set and the
//! `horizon:behaviour` of `assets/<namespace>/particles/<path>.json` give its spawn values) and the
//! provider then moves it every tick. The game resolves a type's provider once, when the particle
//! sprite sets load, so a tick goes to the provider that owns the particle and to no other.
use std::{collections::BTreeMap, sync::Arc};

/// One particle of a provider's type, as its tick reads and writes it: the fields of vanilla's
/// `Particle` a `tick` moves. Plain numbers, so a tick allocates nothing, and one value for
/// native and wasm mods alike.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it for each tick and a mod only edits its fields
#[derive(horizon_mod_api_macros::Wire)]
pub struct ModParticle {
    /// `Particle.x/y/z`: the position now. A tick moves the particle by writing it.
    pub position: [f64; 3],
    /// `Particle.xo/yo/zo`: where it was a tick ago, which a frame interpolates from. The game
    /// sets it to `position` before each tick; a tick that teleports the particle sets it too.
    pub previous: [f64; 3],
    /// `Particle.xd/yd/zd`: the velocity the spawn gave it (the declared one, or the spawn
    /// call's). The game applies no vanilla motion to a provider's particle, so a tick moves
    /// `position` by it, or not.
    pub velocity: [f64; 3],
    /// `Particle.age`, the ticks this particle has run: 1 on its first tick. The game advances it
    /// before each tick.
    pub age: i32,
    /// `Particle.lifetime`: the particle expires on the tick after `age` reaches it. A tick may
    /// change it.
    pub lifetime: i32,
    /// `SingleQuadParticle.quadSize`: the billboard's half-width in blocks.
    pub size: f32,
    /// `Particle.rCol/gCol/bCol/alpha`, each 0 to 1.
    pub rgba: [f32; 4],
    /// The sprite drawn, an index in the sprite set `particles/<path>.json` lists (the last one
    /// when it is past the list).
    pub frame: u32,
    /// `Particle.remove()`: set it to expire the particle before its lifetime is up.
    pub removed: bool,
    /// Four numbers that stay with this particle from one tick to the next, zero when it
    /// spawns.
    pub state: [f64; 4],
}

/// A mod's particle provider: NeoForge's `ParticleProvider`, held by the game for every particle
/// of its type. Particles of a type share one provider, so per-particle data lives in
/// [`ModParticle::state`], not in the provider.
///
/// A tick has no `Host` and no level: the engine ticks particles inside the render crate, which
/// reaches no gameplay. A provider that needs the world reads it in a hook and keeps what it read.
pub trait ParticleProvider: Send + Sync + 'static {
    /// `Particle.tick`: advance `particle` by one tick. The game has set `previous` and `age`
    /// already and expires the particle once `age` passes `lifetime`; the tick sets `position`,
    /// `frame`, `size`, `rgba`, `lifetime` and `removed`. A number that is not finite expires the
    /// particle.
    fn tick(&self, particle: &mut ModParticle);
}

/// The particle provider registry `register_particle_providers` hands out: NeoForge's
/// `RegisterParticleProvidersEvent`.
pub trait ParticleProviders {
    /// `registerSpriteSet(type, provider)`: `provider` moves every particle of the type `name`
    /// (`mymod:spark`), whose sprite set is `assets/mymod/particles/spark.json`. A later
    /// registration for a name replaces the earlier one. A name no sprite set matches is logged
    /// and its particles draw nothing, as any `named` type without its sprite set does.
    fn register(&mut self, name: String, provider: Arc<dyn ParticleProvider>);
}

// no wire: process-local listener or callback storage; trait objects are invoked through host dispatch, not serialized.
/// The game's [`ParticleProviders`]: the registrations by name.
#[derive(Default)]
// exhaustive: its field is private; the game builds it
pub struct ParticleProviderRegistry {
    providers: BTreeMap<String, Arc<dyn ParticleProvider>>,
}

impl ParticleProviders for ParticleProviderRegistry {
    fn register(&mut self, name: String, provider: Arc<dyn ParticleProvider>) {
        self.providers.insert(name, provider);
    }
}

impl ParticleProviderRegistry {
    /// Each type's provider, the last registration for a name winning.
    pub fn into_providers(self) -> BTreeMap<String, Arc<dyn ParticleProvider>> {
        self.providers
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Drift(f64);
    impl ParticleProvider for Drift {
        fn tick(&self, particle: &mut ModParticle) {
            particle.position[0] += self.0;
        }
    }

    fn particle() -> ModParticle {
        ModParticle {
            position: [0.0; 3],
            previous: [0.0; 3],
            velocity: [0.0; 3],
            age: 1,
            lifetime: 20,
            size: 0.1,
            rgba: [1.0; 4],
            frame: 0,
            removed: false,
            state: [0.0; 4],
        }
    }

    #[test]
    fn a_later_registration_for_a_name_replaces_the_earlier_one() {
        let mut registry = ParticleProviderRegistry::default();
        registry.register("mymod:a".into(), Arc::new(Drift(1.0)));
        registry.register("mymod:b".into(), Arc::new(Drift(2.0)));
        registry.register("mymod:a".into(), Arc::new(Drift(3.0)));
        let providers = registry.into_providers();
        assert_eq!(providers.len(), 2);
        let (mut a, mut b) = (particle(), particle());
        providers["mymod:a"].tick(&mut a);
        providers["mymod:b"].tick(&mut b);
        assert_eq!((a.position[0], b.position[0]), (3.0, 2.0));
    }
}
