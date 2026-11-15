//! Window resize hook declaration.

pub trait Hooks {
    /// After a window resize or applying a changed GUI scale, fullscreen, or Unicode font setting.
    /// `width` and `height` are physical pixels; `gui_width` and `gui_height` are scaled GUI pixels.
    /// Fires only when physical or scaled dimensions change.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "WindowResizeEvent",
        cancellable = false
    )]
    fn window_resize();
}
