//! The `registered_player_command` hook.
pub trait Hooks {
    /// A player submitted a command whose root appears in the mod command tree
    /// and whose root permission the player meets. The player's state is in place.
    /// This runs before argument parsing, so listeners that own the root can apply
    /// their own syntax and subcommand permissions. Return `Flow::Cancel` once
    /// handled. Otherwise the command goes through ordinary parsing and resolution.
    /// The text has no leading slash and is untrusted. Console, function and
    /// nested `execute` invocations use the regular command path.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "ran",
        neoforge = "none (CommandEvent runs after parsing)"
    )]
    fn registered_player_command();
}
