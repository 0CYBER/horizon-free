//! The client’s saving-world presentation, separate from normal screen rendering.

pub trait Hooks {
    /// Draw the complete saving-world frame in GUI pixels with `event.draw` and return `Some(())`
    /// to replace the default presentation. To decline, return `None` without drawing. The first
    /// replacement wins. `event.progress` is the existing coarse progress in [0, 1], not a measure
    /// of durable completion; it is `None` while waiting for generation or the integrated server.
    /// Requires a client window and renderer. Fires only on saving-screen redraws, not during
    /// startup, resource reloads, or a quit that has no saving screen. Replacing the frame cannot
    /// change the save, its timeout, or when the client leaves the world or quits.
    #[hook(
        side = "client",
        contract = "replace",
        out = "replacement",
        neoforge = "none"
    )]
    fn closing_save_screen_draw();
}
