//! The `tab_complete` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0333

pub trait Hooks {
    /// A guest asked to complete `command`; owned API `Suggestion` records may be
    /// added to or filtered from `suggestions`. `range` is the replaced byte range.
    /// The game snapshots completions only when a listener subscribes.
    #[hook(
        side = "server",
        contract = "collect",
        out = "suggestions",
        neoforge = "none (NeoForge has no generic tab completion event)"
    )]
    fn tab_complete();
}
