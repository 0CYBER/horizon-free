//! A client mod's line for the narrator (`GameNarrator.saySystemNow` and
//! `saySystemQueued`): when it is said, and why it was not.

use crate::TextFormError;

/// When the narrator says a mod's line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum Narration {
    /// `GameNarrator.saySystemNow`: cuts off what is being said and says the
    /// line at once, as a target change in Jade does.
    Now,
    /// `GameNarrator.saySystemQueued`: says the line after what is being said.
    Queued,
}

/// Why the narrator did not say a mod's line. Nothing was said or logged in
/// any case; each cause has its own name.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum NarrateError {
    /// The component was refused at the boundary (too deep, a bad NBT path).
    Text(TextFormError),
    /// The component's plain text (`Component.getString`) is empty: vanilla
    /// says nothing for it either.
    Empty,
    /// The player's Narrator option does not narrate system messages: it is
    /// Off or Chat only (`NarratorStatus.shouldNarrateSystem`).
    Off,
    /// No speech is available (`Narrator.active` is false: no platform
    /// speech, or it failed to open), no client narrator answers (a server's
    /// thread), or there is no game backend.
    Unavailable,
}

impl From<TextFormError> for NarrateError {
    fn from(error: TextFormError) -> Self {
        Self::Text(error)
    }
}

impl std::fmt::Display for NarrateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Text(error) => write!(f, "the component was refused: {error:?}"),
            Self::Empty => f.write_str("the component's text is empty"),
            Self::Off => f.write_str("the Narrator option does not narrate system messages"),
            Self::Unavailable => f.write_str("no narrator speech is available"),
        }
    }
}

impl std::error::Error for NarrateError {}

impl crate::NoBackend for NarrateError {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}
