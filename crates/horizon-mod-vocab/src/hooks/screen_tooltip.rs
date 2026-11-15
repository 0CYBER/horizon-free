//! The `screen_tooltip` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0030

pub trait Hooks {
    /// The tooltip at `at` (as in `screen_click`) on block `block`'s
    /// screen, a mod's, over no item: push its lines when it is over
    /// something of yours. Legacy colour codes (§) work.
    #[hook(
        side = "client",
        contract = "collect",
        out = "lines",
        scope = "block_entity",
        key = "block",
        neoforge = "none (see screen)"
    )]
    fn screen_tooltip();
}
