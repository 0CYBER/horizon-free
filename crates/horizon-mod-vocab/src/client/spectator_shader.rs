//! Client spectator post-effect registrations.

/// A post effect registered by `Hooks::register_entity_spectator_shaders`: the screen effect
/// spectating an entity of type `entity` puts on, as vanilla's creeper, spider and enderman do.
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: mods construct registrations and the game reads both fields
#[derive(horizon_mod_api_macros::Wire)]
pub struct EntitySpectatorShader {
    /// The spectated entity type (`mymod:wisp`).
    pub entity: String,
    /// The post effect, the chain at `assets/<namespace>/post_effect/<path>.json`
    /// (`mymod:wisp_vision`), as a resource pack or the mod's assets supply it.
    pub shader: String,
}
