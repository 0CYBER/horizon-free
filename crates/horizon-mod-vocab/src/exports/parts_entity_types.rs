//! Exports of `crate::parts` items defined in `parts/entity_types.rs`.
pub use parts::limits as general_entity_limits;
pub use parts::{GeneralEntityCategory, GeneralEntityRegistrationError, GeneralSavePolicy};
mod __private {
    pub use crate::parts::{GeneralEntityTypeDeclaration, general_entity_type_declarations};
}
