//! Stack-valued requests for the server's item particle option.
use crate::{ItemStack, ParticleKey, ParticleOptions};
impl ParticleOptions {
    /// Vanilla `ItemParticleOption(type, stack)` for server delivery.
    /// The receiving client resolves the stack's ground model with seed 0,
    /// using the same path as `ClientHost::item_particle_options`.
    /// A send refuses unknown, invalid or oversized stacks and returns false.
    pub fn item_stack(ty: ParticleKey, stack: ItemStack) -> Self {
        Self {
            item_stack: Some(stack),
            ..Self::simple(ty)
        }
    }
}
