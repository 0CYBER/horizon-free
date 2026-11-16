//! The render groups a client mod's particles draw in, as it registers them in
//! `register_particle_groups` (NeoForge's `RegisterParticleGroupsEvent` and vanilla's
//! `ParticleRenderType`).

/// A render group a native mod registers: vanilla's `ParticleRenderType` and the `ParticleGroup`
/// the engine keeps for it. The particles of a group are counted, admitted, ticked and removed
/// apart from every other group's (each has vanilla's own 16384-particle limit), and drawn after
/// the built-in particles, in registration order.
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: a mod builds it with a literal
#[derive(horizon_mod_api_macros::Wire)]
pub struct ParticleGroup {
    /// `ParticleRenderType.name`: `<namespace>:<path>` in a namespace other than vanilla's, like
    /// `mymod:glow`.
    pub id: String,
    /// How the group's particles blend, whatever their own sprites hold.
    pub blend: ParticleBlend,
    /// The particle types that draw in this group, named as `ParticleOptions::named` names them
    /// (`mymod:spark`). Each is a type a `particles/<path>.json` gives a sprite set, with or
    /// without a `register_particle_providers` provider; a type another group lists already, or
    /// no sprite set matches, is logged and left out.
    pub particles: Vec<String>,
}

/// How a [`ParticleGroup`]'s particles blend: the layer they draw on, which decides their order
/// against the translucent terrain (`SingleQuadParticle.Layer`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: vanilla's two quad layers, matched without a wildcard
#[derive(horizon_mod_api_macros::Wire)]
pub enum ParticleBlend {
    /// `SingleQuadParticle.Layer.OPAQUE`: drawn with the terrain, writing depth; the sprite's
    /// transparent texels are cut out.
    Opaque,
    /// `SingleQuadParticle.Layer.TRANSLUCENT`: drawn after the translucent terrain, blended by
    /// each particle's alpha.
    Translucent,
}
