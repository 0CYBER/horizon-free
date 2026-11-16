//! The `player_game_mode_changed` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0253

pub trait Hooks {
    /// A server player's game mode is about to change (NeoForge's
    /// `PlayerChangeGameModeEvent`): from `from` to `to`, both the shared
    /// `GameMode` used by commands and `PlayerRef`. `player` is the UUID and
    /// `name` the name. Heard for
    /// the `/gamemode` command, the F3+F4 switcher's packet and a hardcore
    /// world's respawn into spectator, once for each change to a different
    /// mode; not for the world's default mode a player joins with. Set `to`
    /// to send the player to another mode instead: listeners run in order and
    /// each sees the mode the one before left. Return `Flow::Cancel`, or
    /// leave `to` at the player's current mode, and the player keeps their
    /// mode (nothing is sent, the command counts it as no change); the
    /// command's message names the mode it asked for, as vanilla's does. For a change the game makes inside
    /// a player's turn their state is swapped in; for the hardcore respawn
    /// it is not.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "PlayerEvent.PlayerChangeGameModeEvent",
        version = 2
    )]
    fn player_game_mode_changed();
}
