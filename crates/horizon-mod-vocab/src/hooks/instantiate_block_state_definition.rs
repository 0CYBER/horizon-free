//! The `instantiate_block_state_definition` hooks: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A blockstate file whose `neoforge:definition_type` a mod registered
    /// (`register_block_state_models`) is read: push a `BlockStateModelPick` for each state of
    /// `states` that draws a model, or one `Refuse` if the file doesn't read, for a type your mod
    /// owns. `config` is the file's JSON object converted to a `Tag`, `block` the block it
    /// belongs to. It is asked once per file on each resource load, on the resource loader,
    /// never per frame.
    #[hook(
        side = "client",
        contract = "collect",
        out = "models",
        context = "registration",
        neoforge = "none (CustomBlockModelDefinition.instantiate of a type RegisterBlockStateModels registers; NeoForge adds no event for it)",
        target = "horizon_client_render::mod_block_state_models::native_instantiate_block_state_definition"
    )]
    fn instantiate_block_state_definition();
}
