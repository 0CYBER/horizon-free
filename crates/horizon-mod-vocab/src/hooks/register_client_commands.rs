//! The `register_client_commands` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0017

pub trait Hooks {
    /// The client's own commands (NeoForge's `RegisterClientCommandsEvent`), which
    /// `command` runs: push a `Command` per usage (`x <mode:word>`).
    /// Asked when a connection starts and again when the server sends its
    /// own tree, so the chat box colours, completes and lists them as the
    /// game's own; a name the game already has keeps that command's
    /// behaviour. A client command has no operator level: one with a nonzero
    /// `level` is left out.
    #[hook(
        side = "client",
        contract = "collect",
        out = "commands",
        neoforge = "RegisterClientCommandsEvent"
    )]
    fn register_client_commands();
}
