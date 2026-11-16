//! The `register_particle_providers` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The code-defined particle providers of a mod's own particle types (NeoForge's
    /// `RegisterParticleProvidersEvent`): `register(name, provider)`, and every particle of the
    /// type `name` (`ParticleOptions::named("mymod:spark")`) is moved each tick by
    /// `ParticleProvider::tick`. The sprite set and the declarative spawn values stay in
    /// `assets/<namespace>/particles/<path>.json`. A later registration for a name replaces the
    /// earlier one; a name no sprite set matches is logged and draws nothing. Asked once at start
    /// on the client, before resources load; the game resolves each type's provider when the
    /// sprite sets load, on every resource load as well.
    #[notes = r#"## Particle providers

A particle provider (NeoForge's `RegisterParticleProvidersEvent` and `ParticleProvider`) gives a mod's own
particle type code. `Hooks::register_particle_providers` registers one provider per type name once at start,
before resources load, and the game resolves each type's provider when the particle sprite sets load, so a
particle tick dispatches to the provider that owns it: no lookup, no scan over the mods.

```rust,ignore
struct Orbit;
impl ParticleProvider for Orbit {
    fn tick(&self, particle: &mut ModParticle) {
        let t = f64::from(particle.age) * 0.4;
        particle.position = [
            particle.state[0] + t.cos(),
            particle.state[1] + f64::from(particle.age) * 0.05,
            particle.state[2] + t.sin(),
        ];
        particle.rgba[3] = 1.0 - particle.age as f32 / particle.lifetime as f32;
        if particle.age > 40 {
            particle.removed = true;
        }
    }
}

fn register_particle_providers(
    &self,
    _context: &RegistrationContext,
    event: &mut RegisterParticleProvidersEvent<'_>,
) {
    event.providers.register("example:orbit".into(), Arc::new(Orbit));
}
```

The particle is built as a declarative one is: `ParticleOptions::named("example:orbit")` draws the sprite set
of `assets/example/particles/orbit.json`, with the spawn values of its `horizon:behaviour` (lifetime, size, alpha,
velocity). The engine keeps the sprite handling and the camera-distance, Particles-setting and population limits;
only the motion is the provider's. Before each `tick` the game advances `age` (1 on the first tick) and sets
`previous`, and it expires the particle once `age` passes `lifetime`, or after a tick that sets `removed` or
leaves a number that is not finite. The particle draws on the blended layer, so `rgba[3]` may fade it, unless a `register_particle_groups`
group lists its type: it then draws in that group, on the group's blend.
`position`, `frame` (a sprite of the set, clamped), `size`, `rgba`, `lifetime` and `removed` are the tick's to
write. `state` is four numbers that stay with the particle, zero at spawn; it is the only per-particle data,
since every particle of a type shares the one provider (the same for native and wasm mods: a wasm tick is one
call with the particle copied in and out). A tick has no `Host` and no level, since the engine ticks particles
below gameplay; a provider that needs the world reads it in a hook. A type with no provider keeps its
declarative motion, and live provider particles are dropped when resources reload, as a compiled
controller's are."#]
    #[hook(
        side = "client",
        contract = "listen",
        context = "registration",
        neoforge = "RegisterParticleProvidersEvent"
    )]
    fn register_particle_providers();
}
