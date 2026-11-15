//! The `screen_key_released_pre` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0120

pub trait Hooks {
    /// Before an open screen handles a named key's release (same screen gate as key presses).
    /// Cancel bypasses screen handling; physical keys and mapped keyboard hold/toggle state release.
    #[hook(
        side = "client",
        contract = "cancel",
        out = "canceled",
        neoforge = "ScreenEvent.KeyReleased.Pre",
        cancellable = true
    )]
    fn screen_key_released_pre();
}
