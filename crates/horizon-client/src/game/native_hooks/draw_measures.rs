use super::*;

/// Helpers for native mods, kept as stable as the hook points.
// Native mods use these; nothing in Horizon does.
#[allow(dead_code)]
impl Game<'_> {
    /// The width of `text` in GUI pixels, as `native_text` draws it on `draw` (`Font.width`).
    pub(in crate::game) fn native_text_width(&self, _draw: &HudBatch, text: &str) -> f32 {
        HudBatch::text_width(&self.gui.atlas, text)
    }
    /// The size of a GUI atlas sprite in its own pixels, if present.
    pub(in crate::game) fn gui_sprite_pixels(&self, name: &str) -> Option<[f32; 2]> {
        let r = self.gui.atlas.sprites.get(name)?;
        Some([r.w as f32, r.h as f32])
    }
}
