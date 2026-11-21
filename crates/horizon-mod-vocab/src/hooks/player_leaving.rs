//! The `player_leaving` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `player` (`name`) is about to leave, with the plain-text `reason` the
    /// connection ended with (a kick's reason, `Disconnected`, `Timed out`, or
    /// `Server closed` when the server stops). Heard before the player is
    /// removed: they are still in `host.players().server_players()` and in
    /// their level, their state is swapped in, and their inventory, open menu
    /// and carried stack are untouched, so a listener can read or clear the
    /// inventory, drop items, kill or send them a last message; what it does
    /// is part of what is saved. Once for each placed network player, before
    /// `player_logged_out`. A fake player and a player held before the
    /// join was announced are not told of, nor is a player whose transfer to
    /// another server is unresolved or committed: their state may already
    /// belong there, so nothing done here could travel with them. A kick is heard by `player_kicked`
    /// first; `player_disconnect` of the leaving player refuses, whatever ended the
    /// connection.
    /// Server-side.
    #[notes = r#"## Player leaving

- **`player_leaving(host, event)`**: the moment before a player is removed, as Paper's `PlayerQuitEvent` fires (the player is still a `Player` in the world), which `player_logged_out` cannot give: it is heard after the player left `server_players()`. A combat-log mod kills a tagged player here; an audit mod reads their inventory. `event.reason` is the plain text the connection ended with. The hook is listen-only: the quit message cannot be changed. It is not heard while the player's transfer to another server is unresolved or committed: their state may already belong there. When the server stops, each player is told here, then by `player_logged_out`, before the server saves."#]
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge's PlayerLoggedOutEvent is fired after the player is removed from the list; the hook listens at Bukkit's PlayerQuitEvent point)",
        paper = "PlayerQuitEvent"
    )]
    fn player_leaving();
}
