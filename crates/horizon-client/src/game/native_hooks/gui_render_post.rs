use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// After the complete normal GUI batch, including screens, prompts, toasts,
    /// debug overlays, the pad cursor and IME. Fires even with F1 or a menu open;
    /// file/world loading and resource-reload overlays do not fire it. The batch
    /// is in GUI pixels. `camera` is this frame's world camera (None on
    /// the panorama). `gui_hidden` is the effective HUD visibility; `focused` is window focus.
    /// A mod decides whether its foreground indicator respects those flags.
    /// Client only; the host is read-only.
    pub(in crate::game) fn native_gui_render_post(
        &self,
        batch: &mut HudBatch,
        gui_hidden: bool,
        focused: bool,
    ) {
        self.fire_gui_render_post(batch, gui_hidden, focused);
    }
}
