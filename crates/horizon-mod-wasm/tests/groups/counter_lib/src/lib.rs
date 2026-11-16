//! A library mod: a capability of its own handler type, which its dependents answer and ask.
use mod_api::{CapabilityKey, Hooks};

/// What `test:counter` answers with.
pub trait Counter: Send + Sync {
    /// Its count.
    fn count(&self) -> u32;
}

/// The capability.
pub const COUNTER: CapabilityKey<dyn Counter> = CapabilityKey::new("test:counter");

#[derive(Default)]
pub struct Library;

#[mod_api::horizon_mod]
impl Hooks for Library {}
