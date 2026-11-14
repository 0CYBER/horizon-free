//! A mob whose AI a mod paused while its physics runs on: why
//! [`MobRef::set_brain_frozen`](crate::MobRef::set_brain_frozen) changed
//! nothing.

/// Why [`MobRef::set_brain_frozen`](crate::MobRef::set_brain_frozen) left the
/// mob as it was. Every refusal changes nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum BrainFreezeRefusal {
    /// The handle is stale, names no entity of the server level in place, or
    /// names a mob taken out for its own tick; also on a client and with no
    /// game backend.
    NoMob,
    /// The handle names an entity of this level that is not a mob the
    /// freeze applies to: a player, an armor stand, an item, or the ender
    /// dragon, whose flight is its phase and not a mob's AI step.
    NotAMob,
}

impl std::fmt::Display for BrainFreezeRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NoMob => "the handle names no mob of this server level",
            Self::NotAMob => "the entity is not a mob whose AI can be paused",
        })
    }
}

impl std::error::Error for BrainFreezeRefusal {}

impl crate::NoBackend for BrainFreezeRefusal {
    fn no_backend() -> Self {
        Self::NoMob
    }
}
