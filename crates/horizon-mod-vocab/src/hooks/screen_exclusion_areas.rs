//! The `screen_exclusion_areas` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The hook `native_screen_exclusion` asks: a mod adds the rectangles it
    /// draws over `screen` (a side panel, a button column) to `areas`, in GUI
    /// pixels, `[x, y, width, height]`. NeoForge has no counterpart in the
    /// game; recipe viewers ask for it through their own API
    /// (`EmiExclusionArea`, JEI's `IGuiContainerHandler.getGuiExtraAreas`).
    /// Client-side.
    #[hook(
        side = "client",
        contract = "collect",
        out = "areas",
        cancellable = false,
        neoforge = "none (NeoForge has no counterpart in the game; recipe viewers define their own API)"
    )]
    fn screen_exclusion_areas();
}
