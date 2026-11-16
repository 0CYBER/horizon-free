//! The `player_commands_sent` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0249

pub trait Hooks {
    /// The command tree a server sends a player (NeoForge's and Paper's
    /// "commands sent to a player" stage): when they join and when their
    /// level changes (`/op`, `/deop`, the LAN settings). `names` are the
    /// root commands their client would get, vanilla's they may use and the
    /// mods'; take a name out and that client gets the tree without it
    /// (and without what it leads to), so it neither completes nor
    /// highlights it. This hides the command from the client only: the
    /// server still runs a typed one, `player_command` stops it.
    /// For a player who is already on, their state is swapped in and the
    /// helpers (`PlayerRef::tell`...) work; for one who is joining it is not, and
    /// their client has no world to tell yet. The tree the hub sends again by
    /// itself (after a respawn or a change of level) keeps the last answer
    /// and does not ask again; the integrated server's owner is not asked
    /// when the world's cheats are switched.
    #[hook(
        side = "server",
        contract = "collect",
        out = "names",
        neoforge = "none (NeoForge 26.3 has no such event)"
    )]
    fn player_commands_sent();
}
