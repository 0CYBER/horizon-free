//! The `parts` module, and exports of the items the generator declares in it (`parts_modules.rs`).
mod parts;
mod __private {
    #[cfg(any(test, feature = "testing"))]
    pub use crate::parts::tests as part_tests;
}
