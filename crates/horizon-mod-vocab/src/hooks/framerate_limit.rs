//! The `framerate_limit` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0219

pub trait Hooks {
    /// The frame rate this window is about to be held to: Max Framerate after the
    /// inactivity throttle and Battery Mode (260 is unlimited). `focused` is whether the
    /// window has focus, `minimized` whether it is minimised and `in_world` whether a world
    /// is open. A mod may lower `limit`, as Dynamic FPS does for a window in the background:
    /// Horizon keeps the lower of its own limit and the mod's, and ignores values below 1.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "none (vanilla FramerateLimitTracker.getFramerateLimit fires no event)",
        cancellable = false
    )]
    fn framerate_limit();
}
