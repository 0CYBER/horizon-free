//! The `customize_boss_event_progress` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// One boss bar is about to be drawn (NeoForge's `CustomizeGuiOverlayEvent.BossEventProgress`,
    /// in `BossHealthOverlay`): `bar` is the bar, `x` and `y` come in as its top-left corner in GUI
    /// pixels (91 left of the screen's centre; 12 for the first bar), and `increment` as the
    /// spacing to the next bar's `y` (19). Move `x` and `y` to draw this bar, and its title, there;
    /// set `increment` to space the next bar, which comes in that far below where this one came in
    /// (moving `y` does not move the bars after it). Set `cancel` and this bar is not drawn; the
    /// increment is still applied, leaving room for what a mod draws in its place. Asked once a
    /// frame per bar, in the order the server added them, until the next bar would start a third
    /// of the way down the screen; not under F1. Client-side.
    #[hook(
        side = "client",
        contract = "cancel",
        out = "cancel",
        neoforge = "CustomizeGuiOverlayEvent.BossEventProgress",
        cancellable = true
    )]
    fn customize_boss_event_progress();
}
