//! Opaque identities for fluid names retained from unavailable content.
use crate::{FluidKey, Host, UnresolvedFluid};
/// A registered fluid, or an opaque retained name from unavailable content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum FluidIdentity {
    /// A registered fluid.
    Resolved(FluidKey),
    /// A name retained verbatim until its declaring mod returns.
    Unresolved(UnresolvedFluid),
}

impl FluidIdentity {
    /// Its registered key, or none when the fluid is unresolved.
    pub fn key(self) -> Option<FluidKey> {
        match self {
            Self::Resolved(key) => Some(key),
            Self::Unresolved(_) => None,
        }
    }
}

impl UnresolvedFluid {
    #[cfg(any(not(target_arch = "wasm32"), test))]
    pub(crate) fn from_slot(slot: u64) -> Result<Self, &'static str> {
        let access = access().ok_or("unresolved fluid storage requires the platform runtime")?;
        (access.slot_is_live)(slot)
            .then_some(Self(slot))
            .ok_or("unknown unresolved fluid")
    }

    /// The original stored name, read from the host's process-lifetime table.
    pub fn id(self, host: &dyn Host) -> String {
        host.unresolved_fluid_id(self)
    }
}

/// Compare an identity with an NBT name without allocating.
pub fn matches_utf16(identity: FluidIdentity, id: &[u16]) -> bool {
    match identity {
        FluidIdentity::Resolved(key) => {
            crate::content::registry_keys::fluid_key_utf16(id) == Some(key)
        }
        FluidIdentity::Unresolved(handle) => lookup_utf16(id) == Some(handle),
    }
}

/// Resolve an identity at an engine-owned boundary.
pub fn identity_name(identity: FluidIdentity) -> String {
    match identity {
        FluidIdentity::Resolved(key) => crate::content::registry_keys::fluid_id(key)
            .expect("resolved fluid key")
            .to_owned(),
        FluidIdentity::Unresolved(handle) => name(handle),
    }
}

// no wire: process-local callback table installed by the platform runtime.
// exhaustive: the hidden installer builds every callback field together.
// hidden: platform-owned instrumentation plumbing, not a mod authoring surface.
#[doc(hidden)]
pub struct UnresolvedFluidAccess {
    pub intern: fn(&str) -> Option<u64>,
    pub lookup: fn(&str) -> Option<u64>,
    pub lookup_utf16: fn(&[u16]) -> Option<u64>,
    pub name: fn(u64) -> String,
    pub slot_is_live: fn(u64) -> bool,
}
fn access() -> Option<&'static UnresolvedFluidAccess> {
    crate::dispatch_scope::backend().map(|backend| &backend.unresolved_fluids)
}
/// Intern at the engine-owned boundary; unavailable storage refuses issuance.
pub fn intern(id: &str) -> Option<UnresolvedFluid> {
    (access()?.intern)(id).map(UnresolvedFluid)
}
/// Find an already-issued identity without allocating.
pub fn lookup(id: &str) -> Option<UnresolvedFluid> {
    (access()?.lookup)(id).map(UnresolvedFluid)
}
/// Find an already-issued NBT identity without allocating.
pub fn lookup_utf16(id: &[u16]) -> Option<UnresolvedFluid> {
    (access()?.lookup_utf16)(id).map(UnresolvedFluid)
}
/// Resolve a handle whose issuing platform storage lives for the process.
pub fn name(fluid: UnresolvedFluid) -> String {
    (access()
        .expect("issued fluid handles require their platform storage")
        .name)(fluid.0)
}
