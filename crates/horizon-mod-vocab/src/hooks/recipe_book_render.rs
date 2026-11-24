//! The `recipe_book_render` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The open stock recipe book has drawn its panel, page and recipe alternatives. Draw here
    /// before the screen's Foreground and deferred tooltips. `rect` is the actual book panel
    /// `[x, y, width, height]` in GUI pixels, excluding tabs; `covers_menu` means the narrow book
    /// covers the container and its slots cannot be hovered. Does not fire for a closed book,
    /// a cancelled screen, or a book owned by a native mod screen.
    #[hook(side = "client", contract = "listen")]
    fn recipe_book_render();
}
