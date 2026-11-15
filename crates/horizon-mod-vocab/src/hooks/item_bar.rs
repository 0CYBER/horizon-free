//! The `item_bar` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0071

pub trait Hooks {
    /// The small bar `GuiGraphicsExtractor.itemDecorations` draws under an
    /// item, wherever the game shows one: `bar` arrives holding vanilla's value
    /// (the damage bar, or a bundle's fullness already applied) and may be
    /// replaced (width `0..=13`, colour `rgb`) or set to `None` to hide it.
    /// Asked each frame a stack is drawn. Client-side.
    #[hook(
        side = "client",
        contract = "listen",
        scope = "item",
        key = "stack",
        neoforge = "none (vanilla Item.isBarVisible/getBarWidth/getBarColor; RegisterItemDecorationsEvent registers decorators, not bar queries)"
    )]
    fn item_bar();
}
