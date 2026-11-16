//! The `register_gui_layers` hooks: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The HUD layers a mod adds among vanilla's (NeoForge's `RegisterGuiLayersEvent`): push a
    /// `GuiLayer` per layer, its id `<mod id>:<path>` and its `GuiLayerPlacement`. Asked once at
    /// start, on the client. `gui_layer_draw` draws each layer, between `gui_layer_pre` and
    /// `gui_layer_post` like a vanilla layer, so other mods can hide it or draw around it.
    #[notes = r#"## HUD layers

`Hooks::register_gui_layers` is the client registration-phase counterpart of NeoForge 26.3's
`RegisterGuiLayersEvent`. It receives a `RegistrationContext`, once at client startup, and
collects a `GuiLayer` per layer: an id `<mod id>:<path>` and a `GuiLayerPlacement`, which is
`BelowAll`, `Below(id)`, `Above(id)` or `AboveAll`, as `registerBelowAll`, `registerBelow`,
`registerAbove` and `registerAboveAll`. `Below` and `Above` name a vanilla layer (the ids
`Hooks::gui_layer_pre` lists) or a layer pushed before this one; order against your own layers,
since other mods' layers depend on load order. Layers are placed in push
order, in mod load order. An id in vanilla's namespace, not
of that form, registered already, or placed against a layer that is not in the list yet is
logged and left out.

Each frame the HUD shows, every layer is drawn in that order. A mod's layer fires
`Hooks::gui_layer_pre` with its id (a cancel hides it), then `Hooks::gui_layer_draw`, then
`Hooks::gui_layer_post`, as every layer does. So another mod
hides or wraps a mod's layer the way it hides or wraps a vanilla one, and replacing a layer
(`replaceLayer`, `wrapLayer`) is a cancel in `gui_layer_pre` and a draw of its own. Every mod
is asked about every id in `gui_layer_draw`, so a mod ignores ids it does not own.

A vanilla layer Horizon does not draw this frame (the crosshair in third person, the mount's
health off a mount) still keeps its place: the mod layers around it are drawn in order, at the
next layer that is. As for vanilla's layers, F1 and a canceled `Hooks::render_gui_pre` hide
every mod layer; a mod that
draws under F1 uses `Hooks::gui_render_post`."#]
    #[hook(
        side = "client",
        contract = "collect",
        out = "layers",
        context = "registration",
        neoforge = "RegisterGuiLayersEvent"
    )]
    fn register_gui_layers();
}
