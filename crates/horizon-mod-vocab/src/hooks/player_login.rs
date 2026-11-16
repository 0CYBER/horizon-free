//! The `player_login` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0259

pub trait Hooks {
    /// A player whose login the server has accepted (NeoForge's
    /// `PlayerNegotiationEvent` stage): the name, whitelist, ban and
    /// server-full checks have passed and the player holds a place, but their
    /// saved state is not loaded and they are not placed in a level yet. Set
    /// `cancel` to refuse them: they are disconnected with `reason` (a plain
    /// "Disconnected" if it is left unset) and nothing else of the join (the
    /// join message, `player_logged_in`) happens. Once for each player
    /// who joins, on a server. The player is not placed yet and their client
    /// has no world to show a message in, so no player's state is swapped in
    /// and `PlayerRef::tell` reaches nobody: the refusal `reason` is the answer.
    /// The checks that need no mod (name, whitelist, bans, a full server,
    /// the proxy's forwarding) refuse earlier, on the connection, and this
    /// hook does not hear those players.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (Bukkit's PlayerLoginEvent; NeoForge's PlayerNegotiationEvent is not cancellable)",
        cancellable = true
    )]
    fn player_login();
}
