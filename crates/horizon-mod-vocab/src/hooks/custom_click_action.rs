//! The `custom_click_action` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `CustomClickActionEvent`: a server player's client sent a `custom`
    /// click action (a dialog button or a text component's click event): its id and
    /// optional payload. Vanilla only logs it; set `cancel` to skip that. Only play-phase
    /// clicks are reported: one sent during configuration (no player yet) is not. The
    /// player is not swapped in. Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "CustomClickActionEvent",
        cancellable = true
    )]
    fn custom_click_action();
}
