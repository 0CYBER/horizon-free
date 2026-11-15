//! Stack-resolved client particle reads.
#![allow(unused_variables)]
use crate::*;

/// Item particle model resolution on the client.
#[horizon_mod_api_macros::host_area]
pub trait ItemParticlesClientHost {
    /// Resolve vanilla's `ItemParticleOption` for this stack using the ground
    /// display context and seed 0, exactly like the game's eating particles.
    /// Includes every resolved model layer's particle sprite, respecting resource
    /// packs and stack component overrides. Returns `None` for an invalid stack
    /// or unavailable resources/provider. Available in Setup, Reloading and World.
    /// Resolve again after a resource reload; this read allocates the sprite list.
    #[access(PublishedResources)]
    fn item_particle_options(&self, stack: &ItemStack) -> Option<ParticleOptions>;
}
