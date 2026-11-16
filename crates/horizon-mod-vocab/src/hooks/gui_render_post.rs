//! The `gui_render_post` declarations reconciled with the fork API.

pub trait Hooks {
    /// After the complete normal GUI batch, including screens, prompts, toasts,
    /// debug overlays, the pad cursor and IME. Fires even with F1 or a menu open;
    /// file/world loading and resource-reload overlays do not fire it. The batch
    /// is in GUI pixels. `native_camera` is this frame's world camera (None on
    /// the panorama). `gui_hidden` is the F1 setting; `focused` is window focus.
    /// A mod decides whether its foreground indicator respects those flags.
    /// Client only; the host is read-only. API @API:gui_render_post@.
    #[notes = r#"## Foreground GUI and client lifecycle

`Hooks::gui_render_post` runs after the frame's GUI, including menus and the
F1-hidden HUD. Its `batch` is a `DrawContext`, not an engine `HudBatch`; use
its drawing methods and `size()` to place overlays. `gui_hidden` and `focused`
let a listener choose whether to draw. The GUI paint, nine-slice, and tooltip
operations also use this drawing context and the window's existing atlas.
`Hooks::menu_background` can adjust the requested blur radius or set it to
`None` before the game validates it.

`Hooks::entity_outline` receives the entity id, kind, and mutable optional RGB
outline for the current client render. Leave the existing value unchanged to
preserve vanilla glowing; `None` disables it. These read-only client hooks
receive `&dyn ClientHost`. They do not grant server writes.

`Hooks::client_disconnecting` runs before an actual connection is discarded,
not for a dimension change. Initialization and final shutdown still use the
existing `client_setup` and `client_stopping` hooks, with no parallel aliases."#]
    #[hook(
        side = "client",
        contract = "listen",
        cancellable = false,
        neoforge = "RenderGuiEvent.Post"
    )]
    fn gui_render_post();
}
