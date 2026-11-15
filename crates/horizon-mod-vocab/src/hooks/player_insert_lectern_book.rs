//! The `player_insert_lectern_book` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0254

pub trait Hooks {
    /// A server player inserted a book into a lectern, after the block entity and inventory changed.
    /// `book` is the item id. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_insert_lectern_book();
}
