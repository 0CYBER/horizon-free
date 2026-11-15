//! Load-owned game-event identities.
use crate::{Host, NoBackend};
/// An entry of the startup `minecraft:game_event` registry. Reloading world data
/// preserves this identity; it belongs to the installed mod load.
// no wire: slot_keys! encodes the u32 and validates it against the current host registry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct GameEventKey(pub(crate) u32);
/// Why a retained game-event identity cannot be read here.
#[derive(Clone, Copy, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
#[non_exhaustive]
pub enum GameEventKeyError {
    /// No registry backend is available.
    Unavailable,
    /// The slot is unknown in the current registry table.
    InvalidKey,
}
impl NoBackend for GameEventKeyError {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}
impl GameEventKey {
    /// Read the exact namespaced id, refusing an unknown slot without panicking.
    pub fn id(self, host: &dyn Host) -> Result<String, GameEventKeyError> {
        crate::backend_of(host)
            .ok_or(GameEventKeyError::Unavailable)?
            .game_event_key_id(self)
    }
}
#[cfg(test)]
#[path = "game_event_key_tests.rs"]
mod tests;
