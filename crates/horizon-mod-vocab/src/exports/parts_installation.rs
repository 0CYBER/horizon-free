//! Implementation-only owned metadata crossing from authoring to lookup storage.
mod __private {
    // hidden: platform-owned instrumentation plumbing, not a mod authoring surface.
    #[doc(hidden)]
    pub use crate::parts::installation as parts_installation;
    pub use crate::parts::{RegistrationAccess, RegistrationKeyKind};
    pub use crate::parts::{owned_declaration_id, owns_namespace, qualified};
    pub use crate::runtime::entered_registration;
    pub use crate::world::dimension::NOISE_GENERATOR;
}
