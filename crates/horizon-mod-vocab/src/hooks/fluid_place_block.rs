//! The `fluid_place_block` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Liquid at `liquid_pos` is about to place `state` at `pos`. Set
    /// `cancel` to prevent placement or change `new_state` to replace it.
    /// Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "BlockEvent.FluidPlaceBlockEvent",
        cancellable = true,
        target = "horizon_client::game::native_hooks::fluid_place_block::GameLevel<'_>::native_fluid_place_block"
    )]
    fn fluid_place_block();
}
