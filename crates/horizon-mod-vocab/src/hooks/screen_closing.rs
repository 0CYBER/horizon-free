//! The `screen_closing` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The `screen` that was open is being replaced (`ScreenEvent.Closing`), after `screen_opening` let it.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "ScreenEvent.Closing",
        cancellable = false
    )]
    fn screen_closing();
}
