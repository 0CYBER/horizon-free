//! The `register_commands` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0032

pub trait Hooks {
    /// The commands native mods add to the server's command tree: push
    /// yours. Asked once when a world opens to players (a dedicated
    /// server, a LAN host, single player's server thread); every player's
    /// chat box then completes them as vanilla's, within their level. A
    /// name vanilla has is left out. Asked again after a mod calls
    /// `host.commands().refresh_commands()`. Each executable usage keeps its own
    /// permission level; a lower-level child only opens traversal through a
    /// shared prefix, not execution of that prefix. An argument's kind is one
    /// of `word`, `text`, `integer`, `number`, `boolean`, `player`, `item`,
    /// `block_pos` and `vec3`, or a command argument type a mod registered
    /// (`Registrar::register_command_argument`), named by its namespaced id.
    #[hook(
        side = "server",
        contract = "collect",
        out = "commands",
        neoforge = "RegisterCommandsEvent"
    )]
    fn register_commands();
}
