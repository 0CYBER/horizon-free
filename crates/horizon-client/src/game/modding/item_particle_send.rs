//! Validation of stack-valued particle sends after the API stack boundary.
use horizon_client::particles::{ParticleOptions, ParticleType};
pub(super) fn supported(options: &ParticleOptions) -> bool {
    options.ty == ParticleType::Item
        && options
            .item_stack
            .as_ref()
            .is_some_and(|stack| stack.count > 0)
}
