//! The `screen_render_stages` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Before the open screen draws itself (`ScreenEvent.Render.Pre`, vanilla's `Screen.renderWithTooltips`): draw to put something under it, or set `canceled` and the screen is not drawn, and neither is its `screen_render_background` or `screen_render_foreground`. The world, the HUD layers and the toasts still draw, and `screen_render_post` still fires. Fires once a frame on the window, for the screens `screen_render_post` covers: a screen of the game's, the title's own screens, Win and Options. Not while playing with no screen, nor for a server dialog, the server pack, link and code-of-conduct prompts, a book or a sign editor drawn over gameplay. Draw with `native_fill`, `native_sprite` and `native_text`; `screen.rect` is the screen's container panel (or the whole window).
    #[hook(
        side = "client",
        contract = "cancel",
        out = "canceled",
        neoforge = "ScreenEvent.Render.Pre",
        cancellable = true
    )]
    fn screen_render_pre();
    /// The open screen is about to draw its contents (`ScreenEvent.Render.Background`, after vanilla's `renderBackground` and before `render`): draw here for what must sit under the screen's widgets, items and tooltips. Not cancellable. The screen has drawn its background panel; its widgets, slots and items draw over this hook. Screens without a panel, such as the title and chat, fire it before their contents. It does not fire when `screen_render_pre` cancelled. Fires once a frame on the window, for the screens `screen_render_post` covers: a screen of the game's, the title's own screens, Win and Options. Not while playing with no screen, nor for a server dialog, the server pack, link and code-of-conduct prompts, a book or a sign editor drawn over gameplay. Draw with `native_fill`, `native_sprite` and `native_text`; `screen.rect` is the screen's container panel (or the whole window).
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "ScreenEvent.Render.Background",
        cancellable = false
    )]
    fn screen_render_background();
    /// The open screen has drawn its contents (`ScreenEvent.Render.Foreground`, vanilla's point before the deferred tooltips and the preedit overlay). Not cancellable. Tooltips, the dragged stack and the preedit overlay draw over this hook; use `screen_render_post` for drawing over the screen's tooltips. It does not fire when `screen_render_pre` cancelled. Fires once a frame on the window, for the screens `screen_render_post` covers: a screen of the game's, the title's own screens, Win and Options. Not while playing with no screen, nor for a server dialog, the server pack, link and code-of-conduct prompts, a book or a sign editor drawn over gameplay. Draw with `native_fill`, `native_sprite` and `native_text`; `screen.rect` is the screen's container panel (or the whole window).
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "ScreenEvent.Render.Foreground",
        cancellable = false
    )]
    fn screen_render_foreground();
}
