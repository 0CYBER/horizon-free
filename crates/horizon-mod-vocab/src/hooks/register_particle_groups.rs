//! The `register_particle_groups` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The render groups a mod's particles draw in (NeoForge's `RegisterParticleGroupsEvent`): push a
    /// `ParticleGroup` per group, its id `<mod id>:<path>`, how it blends and the particle types in
    /// it. Asked once at start, on the client, before resources load; the game resolves each
    /// type's group when the particle sprite sets load, on every resource load as well. An id in
    /// vanilla's namespace, not of that form, or registered already is logged and left out.
    #[notes = r#"## Particle groups

`Hooks::register_particle_groups` is the client registration-phase counterpart of NeoForge
26.3's `RegisterParticleGroupsEvent`. It receives a `RegistrationContext`, once at client
startup, and collects a `ParticleGroup` per render group: an id `<mod id>:<path>`, a
`ParticleBlend` (`Opaque` or `Translucent`) and the particle types that draw in it.

```rust,ignore
fn register_particle_groups(
    &self,
    _context: &RegistrationContext,
    groups: &mut Vec<ParticleGroup>,
) {
    groups.push(ParticleGroup {
        id: "example:glow".into(),
        blend: ParticleBlend::Translucent,
        particles: vec!["example:orbit".into(), "example:spark".into()],
    });
}
```

A group is vanilla's `ParticleRenderType` with the `ParticleGroup` the particle engine keeps for
it. Its particles are counted, admitted, ticked and removed apart from every other group's, so
each group has vanilla's own limit of 16384 live particles and a crowd in one cannot crowd out
another, and they are drawn after the built-in particles, groups in registration order. The group
decides the layer of its particles (`blend`), whatever their sprites hold. A type is a particle
type `particles/<path>.json` gives a sprite set (`ParticleOptions::named("example:spark")`), with
or without a `register_particle_providers` provider, so a group adds no particle look or motion
of its own: that stays in the declaration and the provider. A type no group lists draws in the
built-in group as before, and the built-in particles are never moved. A type another group lists
already, or no sprite set matches, is logged and left out; an id in vanilla's namespace, not of
that form, or registered already is logged and the group is left out.

The game resolves a type's group once, when the particle sprite sets load (on every resource
load), and each particle carries its group's handle, so a spawn, a tick and a draw look nothing
up by name and scan no mod. Live provider-driven particles are dropped when resources reload, in
a group as in the built-in one."#]
    #[hook(
        side = "client",
        contract = "collect",
        out = "groups",
        context = "registration",
        neoforge = "RegisterParticleGroupsEvent"
    )]
    fn register_particle_groups();
}
