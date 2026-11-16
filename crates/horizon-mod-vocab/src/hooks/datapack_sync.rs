//! The `datapack_sync` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0123

pub trait Hooks {
    /// Collect recipe types once the joining player exists, after admission
    /// and before that player's recipes are sent. On reload,
    /// asked once per online player, at most once per player per reload,
    /// before their recipes are re-sent. With nobody online this
    /// hook is not asked. `player` always identifies the recipient.
    /// Configuration tags precede admission. Only this player's Horizon
    /// client receives the selected recipes; vanilla clients receive nothing
    /// new. An empty answer retains the client's own recipes, or restores
    /// them after an earlier nonempty answer.
    #[hook(
        side = "server",
        contract = "collect",
        out = "recipe_types",
        neoforge = "OnDatapackSyncEvent"
    )]
    fn datapack_sync(player: PlayerRef, recipe_types: &mut Vec<String>);
}
