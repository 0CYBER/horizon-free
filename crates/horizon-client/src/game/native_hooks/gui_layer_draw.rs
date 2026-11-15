use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// Draw one HUD layer a mod registered (`native_register_gui_layers`), between its
    /// `native_gui_layer_pre` and `native_gui_layer_post`; not asked when a mod hid it.
    pub(in crate::game) fn native_gui_layer_draw(&self, layer: &str, draw: &mut HudBatch) {
        self.fire_gui_layer_draw(layer, draw);
    }
}
