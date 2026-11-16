//! The `player_logged_out` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0068

pub trait Hooks {
    /// `player` (`name`) left the game (NeoForge's `PlayerLoggedOutEvent`), by
    /// any path: a lost connection, a kick, the server stopping or failing.
    /// Once for each player `player_logged_in` told of, and also for a kicked
    /// fake player without a preceding login. Silent `remove_fake` sends no logout.
    /// They've already
    /// left `host.players().server_players()`, except when the server stops, which tells
    /// before it saves, with them still listed and able to get a last message.
    /// Server-side.
    #[hook(
        side = "server",
        version = 2,
        contract = "listen",
        neoforge = "PlayerEvent.PlayerLoggedOutEvent"
    )]
    fn player_logged_out();
}
