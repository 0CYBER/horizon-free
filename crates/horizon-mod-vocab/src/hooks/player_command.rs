//! The `player_command` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0107

pub trait Hooks {
    /// A command a player typed (NeoForge's `CommandEvent`, before
    /// `Commands.performCommand` runs it), without its slash: any command,
    /// vanilla's and the mods' own, also one the player may not run. Set
    /// `canceled` to stop it: nothing runs and nothing is answered but the
    /// lines pushed to `replies`, which go to the player as system chat.
    /// No player's state is swapped in and the server's player list is busy
    /// with this one, so `PlayerRef::tell` and the other player helpers
    /// reach nobody here: answer through `replies`. Once for each command a
    /// player sends, on a server, and for each line of a quiet command batch
    /// (`horizon:quiet_commands`, whose replies are dropped). A command the
    /// server refuses before reading it (unsigned chat arguments, the chat
    /// rate limit) never reaches it.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "canceled",
        neoforge = "CommandEvent",
        cancellable = true
    )]
    fn player_command();
}
