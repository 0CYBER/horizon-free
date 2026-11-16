//! The `player_logged_in` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0067

pub trait Hooks {
    /// `player` (`name`) joined the game (NeoForge's
    /// `PlayerLoggedInEvent`): placed in the world and listed in
    /// `host.players().server_players()`, so `PlayerRef::send_payload` reaches them.
    /// Once a join, with no player's state swapped in. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "PlayerEvent.PlayerLoggedInEvent"
    )]
    fn player_logged_in();
}
