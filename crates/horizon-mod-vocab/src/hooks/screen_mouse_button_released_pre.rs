//! The `screen_mouse_button_released_pre` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0094

pub trait Hooks {
    /// A mouse button went up over the open screen
    /// (`ScreenEvent.MouseButtonReleased.Pre`, vanilla's
    /// `Screen.mouseReleased`), with the same `screen`, `button` and
    /// `modifiers` as `screen_mouse_button_pressed_pre` and fired at
    /// the same point. Set `canceled` and the release goes no further: a
    /// pressed slot keeps what it took and no drag, slider or scrollbar lets
    /// go.
    ///
    /// Cancel a release only when the press it ends was cancelled too: a
    /// release the screen took ends the drag or the scroll it began, as it
    /// does in vanilla, and no guard here un-cancels it. Same screens as the
    /// press; a press that opened another screen ends on that one.
    #[hook(
        side = "client",
        contract = "cancel",
        out = "canceled",
        neoforge = "ScreenEvent.MouseButtonReleased.Pre",
        cancellable = true
    )]
    fn screen_mouse_button_released_pre();
}
