//! The `gui_layer_post` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0145

pub trait Hooks {
    /// After one of vanilla's HUD layers (or after the layer a mod hid), with
    /// the ids of `gui_layer_pre`: draw here to put something above
    /// the layer (`RenderGuiLayerEvent.Post`).
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "RenderGuiLayerEvent.Post",
        cancellable = false
    )]
    fn gui_layer_post();
}
