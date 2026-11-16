//! Exports of `crate::parts` items defined in `parts/datapack_registries.rs`.
mod __private {
    pub use crate::parts::datapack_registry_declarations;
    #[cfg(any(test, feature = "testing"))]
    pub use crate::parts::install_process_datapack_registry_declarations;
}
