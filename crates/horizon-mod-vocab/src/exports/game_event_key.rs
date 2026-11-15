pub use content::game_event_key::{GameEventKey, GameEventKeyError};
mod __private {
    pub use crate::content::registry_keys::{
        game_event_key, game_event_key_id, resolve_game_events,
    };
}
