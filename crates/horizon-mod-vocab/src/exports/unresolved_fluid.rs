pub use block::unresolved_fluid::FluidIdentity;
mod __private {
    pub use crate::block::unresolved_fluid::{
        UnresolvedFluidAccess, identity_name as fluid_identity_name,
        intern as intern_unresolved_fluid, lookup as lookup_unresolved_fluid,
        lookup_utf16 as lookup_unresolved_fluid_utf16,
        matches_utf16 as fluid_identity_matches_utf16, name as unresolved_fluid_name,
    };
}
