//! Wire resource identity layout and engine-owned access callbacks.
use super::{Id, WireError};
use std::any::{Any, TypeId};
use std::sync::OnceLock;
// exhaustive: wasm boundary plumbing that generated code builds field by field
// hidden: wasm boundary plumbing used by generated code, not a mod's API
/// A deliberate refusal to move an erased Rust capability between link groups.
/// Typed resources never use this marker; their zero identity is still stale.
#[doc(hidden)]
#[derive(Debug)]
// no wire: resource-table refusal sentinel, not an owned data value.
pub struct Unsupported {
    pub reason: &'static str,
}
impl Unsupported {
    /// Slots never allocates generation zero. Only the erased Any view interprets it.
    pub const ID: Id = Id(0);
    pub const CROSS_GROUP: Self = Self {
        reason: "cross-group capability",
    };
    pub fn shared() -> std::sync::Arc<dyn Any + Send + Sync> {
        static MARKER: std::sync::OnceLock<std::sync::Arc<Unsupported>> =
            std::sync::OnceLock::new();
        MARKER
            .get_or_init(|| std::sync::Arc::new(Self::CROSS_GROUP))
            .clone()
    }
}

/// Platform-owned access to resources across the Wire boundary.
// hidden: platform-owned instrumentation plumbing, not a mod authoring surface.
#[doc(hidden)]
// exhaustive: the platform or fixture constructs this process-local record as a complete unit.
// no wire: process-local scope ownership or callbacks cannot cross a mod boundary.
pub struct ResourceAccess {
    pub intern: fn(&str) -> Result<&'static str, WireError>,
    pub store: fn(Box<dyn Any>) -> Result<Id, WireError>,
    pub load: fn(Id, TypeId) -> Result<Box<dyn Any>, WireError>,
    pub refused: fn(&str),
}
static ACCESS: OnceLock<ResourceAccess> = OnceLock::new();
/// Install once. Unregistered reads refuse but do not seal this socket, so an
/// application can establish a resource scope and retry after an earlier refusal.
// hidden: platform-owned instrumentation plumbing, not a mod authoring surface.
#[doc(hidden)]
pub fn install_resource_access(access: ResourceAccess) -> Result<(), &'static str> {
    ACCESS
        .set(access)
        .map_err(|_| "resource access is already installed")
}
pub(super) fn refused(handler: &str) {
    if let Some(access) = ACCESS.get() {
        (access.refused)(handler);
    }
}
pub(super) fn store(value: Box<dyn Any>) -> Result<Id, WireError> {
    let access = ACCESS
        .get()
        .ok_or(WireError::Invalid("no resource context"))?;
    (access.store)(value)
}
pub(super) fn load<T: 'static>(id: Id) -> Result<T, WireError> {
    let access = ACCESS
        .get()
        .ok_or(WireError::Invalid("no resource context"))?;
    (access.load)(id, TypeId::of::<T>())?
        .downcast::<T>()
        .map(|value| *value)
        .map_err(|_| WireError::Invalid("wrong resource view"))
}

/// Delegate static-string lifetime storage without allocating in the vocabulary.
pub(super) fn intern(value: &str) -> Result<&'static str, WireError> {
    let access = ACCESS.get().ok_or(WireError::Invalid(
        "static string interning requires the platform runtime",
    ))?;
    (access.intern)(value)
}
