//! Startup game-event registry reads.
use super::Game;
impl Game<'_> {
    pub(in crate::game) fn game_event_key(&self, id: &str) -> Option<mod_api::GameEventKey> {
        mod_api::__private::game_event_key(id)
    }
    pub(in crate::game) fn game_event_key_id(
        &self,
        key: mod_api::GameEventKey,
    ) -> Result<String, mod_api::GameEventKeyError> {
        mod_api::__private::game_event_key_id(key).ok_or(mod_api::GameEventKeyError::InvalidKey)
    }
}
