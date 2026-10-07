//! Synchronized float storage using boxed Java Float equality.
//! The scalar implementation also supports bytes; full entity schemas and
//! heterogeneous serializer storage remain separate.
pub use crate::synced_values::{DataError, Observer};
pub type FloatValue = crate::synced_values::DataValue<f32>;
pub type SyncedFloats = crate::synced_values::SyncedValues<f32>;
