//! The `register_fluid_models` hook: a declaration the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The models of your world fluids (NeoForge's `RegisterFluidModelsEvent`): push a
    /// `mod_api::FluidModel` per fluid whose look doesn't say all of it. Its
    /// sprites are block atlas textures, baked at every resource load from that pack's files; the
    /// tint stays the look's. A fluid without one draws its look's sprite on every surface, or
    /// water's textures when it has none. Asked at startup and on resource reload before baking, and
    /// only on the client.
    #[hook(
        side = "client",
        contract = "collect",
        out = "models",
        context = "registration",
        neoforge = "RegisterFluidModelsEvent"
    )]
    fn register_fluid_models();
}
