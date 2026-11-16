//! The `player_lectern_page_change` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0257

pub trait Hooks {
    /// A server player changed the page of a lectern, after its page and redstone pulse changed.
    /// `page` is the zero-based new page. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_lectern_page_change();
}
