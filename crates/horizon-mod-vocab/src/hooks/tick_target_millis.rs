//! The `tick_target_millis` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// How many milliseconds a client tick lasts (`Minecraft.getTickTargetMillis`, the target the
    /// window's `DeltaTracker.Timer` divides the frame's time by): `millis` comes in as vanilla's
    /// (50 at 20 ticks a second, longer when the server's `/tick rate` is slower); set it to speed
    /// up or slow down the client's fixed-step clock, a "timer". Asked once a frame, in menus too,
    /// before the frame's client ticks run (at most ten a frame). An answer that is not a finite
    /// number above zero keeps vanilla's. Only the window's clock changes: a server still ticks
    /// at its own rate. Client-side. Beyond NeoForge: there is no counterpart. Diverges from
    /// upstream: new hook.
    #[hook(
        side = "client",
        contract = "listen",
        cancellable = false,
        neoforge = "none (vanilla Minecraft.getTickTargetMillis; NeoForge adds no event or extension for it)"
    )]
    fn tick_target_millis();
}
