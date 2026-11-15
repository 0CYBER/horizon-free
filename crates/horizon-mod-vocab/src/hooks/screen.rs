//! The `screen` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0027

pub trait Hooks {
    /// A menu opens on this client for block `block`, a mod's: set `screen`
    /// when the block is yours, to lay its screen out (`screen_draw`
    /// draws it). The server asks it too: a slot it hides is not in the menu
    /// there either, so shift-clicks never fill it.
    #[hook(
        side = "both",
        contract = "replace",
        out = "screen",
        scope = "block_entity",
        key = "block",
        host = "read",
        neoforge = "none (Horizon declares a block's screen through hooks; NeoForge registers a Screen class per menu with RegisterMenuScreensEvent)"
    )]
    fn screen();
}
