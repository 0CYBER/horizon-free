//! The `register_commands` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0032

pub trait Hooks {
    /// The commands native mods add to the server's command tree: push
    /// yours. Asked whenever the server builds its command tree, as the reference loader's
    /// `Commands` does: when a world opens (a dedicated server, a LAN host, single player's
    /// server thread), before its functions load, so a function line may name your command;
    /// on each `/reload` and data pack change, so a command you stop pushing is refused by the
    /// next functions loaded; and after a mod calls `host.commands().refresh_commands()`.
    /// Every player's chat box completes them as vanilla's, within their level. A
    /// name vanilla has is left out. Each executable usage keeps its own
    /// permission level; a lower-level child only opens traversal through a
    /// shared prefix, not execution of that prefix. An argument's kind is one
    /// of `word`, `text`, `integer`, `number`, `boolean`, `player`, `item`,
    /// `block_pos` and `vec3`, or a command argument type a mod registered
    /// (`Registrar::register_command_argument`), named by its namespaced id.
    #[notes = r#"## Command argument kinds beyond the basics

- Besides `word`, `text`, `integer`, `number`, `boolean`, `player`, `item` and `block_pos`, an argument's kind may be `vec3`, `vec3_uncentered`, `column_pos`, `rotation`, `dimension` or `nbt`. Each is sent to clients as vanilla's parser, so `~`, `^` and the dimension list complete as in a vanilla command. A `vec3` and a `vec3_uncentered` cannot share a position in one command's usages, as they look the same to a client."#]
    #[hook(
        side = "server",
        contract = "collect",
        out = "commands",
        neoforge = "RegisterCommandsEvent",
        version = 2
    )]
    fn register_commands();
}
