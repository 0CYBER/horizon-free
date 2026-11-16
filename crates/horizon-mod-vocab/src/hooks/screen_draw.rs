//! The `screen_draw` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0028

pub trait Hooks {
    /// Draw block `block`'s screen at `origin` (its top left, GUI pixels),
    /// over a plain panel with its slots' frames and under the items and
    /// labels, from the menu's data slots `data` (the block entity's `data`
    /// on the server): data slots are 16-bit on the wire as in vanilla (a
    /// bigger value wraps), and `data` is empty until the server's first
    /// values arrive, so read it with `get`. Draw with `native_sprite`, `native_fill` and
    /// `native_text`.
    #[hook(
        side = "client",
        contract = "listen",
        scope = "block_entity",
        key = "block",
        neoforge = "none (see screen)"
    )]
    fn screen_draw();
}
