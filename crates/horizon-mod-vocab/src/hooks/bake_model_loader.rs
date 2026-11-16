//! The `bake_model_loader` hooks: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A model whose `loader` a mod registered (`register_model_loaders`) is baked: push its
    /// quads to `quads` for a loader your mod owns. `config` is the model's JSON object converted
    /// to a `Tag`, and `model` names the model. It is asked once per model on each resource load,
    /// on the resource loader, never per frame.
    #[hook(
        side = "client",
        contract = "collect",
        out = "quads",
        context = "registration",
        neoforge = "none (vanilla UnbakedGeometry.bake of a loader ModelEvent.RegisterLoaders registers; NeoForge adds no event for it)",
        target = "horizon_client_render::mod_model_loaders::native_bake_model_loader"
    )]
    fn bake_model_loader();
}
