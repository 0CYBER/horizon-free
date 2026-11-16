//! The `create_fluid_source` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A fluid source at `pos` (`state`) is about to count towards turning
    /// the flowing fluid beside it into a source (NeoForge's
    /// `CreateFluidSourceEvent`, `FlowingFluid.getNewLiquid`): two counted
    /// sources beside a block over a solid block or a source make it one.
    /// Asked for each horizontal neighbour source, whenever the game works
    /// out a block's new fluid, spread checks included: answer fast.
    /// `can_convert` starts as the fluid's rule (`water_source_conversion`
    /// or `lava_source_conversion`). Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "CreateFluidSourceEvent",
        target = "horizon_client::game::native_hooks::create_fluid_source::GameLevel<'_>::native_create_fluid_source"
    )]
    fn create_fluid_source();
}
