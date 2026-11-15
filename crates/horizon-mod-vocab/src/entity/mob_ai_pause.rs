//! A mob whose AI step a mod paused while its physics runs on: why
//! [`MobRef::set_ai_paused`](crate::MobRef::set_ai_paused) changed nothing.

/// Why [`MobRef::set_ai_paused`](crate::MobRef::set_ai_paused) left the mob as
/// it was. Every refusal changes nothing; each case has its own name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum MobAiPauseRefusal {
    /// The handle is from another world epoch or names no entity of the
    /// server level in place.
    Stale,
    /// Asked on a client, or with no game backend: the pause is server side.
    Client,
    /// The mob is held out of the level for its own tick or a hook on it.
    HeldOut,
    /// The Ender Dragon or one of its parts: its flight is its phase, and
    /// there is no AI step to pause.
    NoAiStep,
    /// The entity is not a mob: a player, an armor stand, an item.
    NotAMob,
}

impl std::fmt::Display for MobAiPauseRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Stale => "the handle names no entity of this server level",
            Self::Client => "a mob's AI is paused on the server only",
            Self::HeldOut => "the mob is held out of the level just now",
            Self::NoAiStep => "the dragon and its parts have no AI step to pause",
            Self::NotAMob => "the entity is not a mob",
        })
    }
}

impl std::error::Error for MobAiPauseRefusal {}

impl crate::NoBackend for MobAiPauseRefusal {
    fn no_backend() -> Self {
        Self::Client
    }
}
