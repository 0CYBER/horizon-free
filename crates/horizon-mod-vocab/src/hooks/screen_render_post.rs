//! The `screen_render_post` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0091

pub trait Hooks {
    /// The open screen has drawn itself and its tooltips (`Screen.render`,
    /// the point of `ScreenEvent.Render.Post`): draw over it with
    /// `native_fill`, `native_sprite` and `native_text`, `screen.rect` being
    /// its container panel (or the whole window) and `screen.id` the screen
    /// it is. Once a frame on the window, never while playing with no screen;
    /// `&self`, so it reads and draws only. The Win and Options screens call
    /// it on their own path, after their drawing.
    ///
    /// It does not fire for the screens drawn over gameplay after it (a
    /// server dialog, the server pack, link and code-of-conduct prompts, a
    /// book, a sign editor): those keep `Screen::Playing`, which it skips (the
    /// server pack prompt hides an open screen too, and that screen does not
    /// fire under it), and draw over what it drew, while vanilla's
    /// `ScreenEvent.Render.Post` fires for them. The title's own screens
    /// (`title`, `world_select`, `world_create`, `multiplayer`…) fire it too,
    /// their `screen.id` from the title state machine and the whole window
    /// their `rect`, and no dialog or prompt over them.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "ScreenEvent.Render.Post",
        cancellable = false
    )]
    fn screen_render_post();
}
