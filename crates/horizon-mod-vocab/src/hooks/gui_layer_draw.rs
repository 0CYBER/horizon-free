//! The `gui_layer_draw` hooks: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Draw one HUD layer a mod registered (`register_gui_layers`), each frame the HUD shows,
    /// between that layer's `gui_layer_pre` and `gui_layer_post`; not asked when a mod's
    /// `gui_layer_pre` hid it. `layer` is the layer's id and every mod is asked about every id,
    /// so a mod ignores ids it does not own. Draw with the drawing context; `draw.size()` is
    /// the GUI size. Client-side.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "GuiLayer.render",
        cancellable = false
    )]
    fn gui_layer_draw();
}
