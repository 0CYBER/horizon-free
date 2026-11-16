//! The `grindstone_place` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A grindstone at `pos` made its result after its inputs changed, for
    /// the server player `player` (UUID) and `player_entity_id`
    /// (`GrindstoneEvent.OnPlaceItem`, in `GrindstoneMenu.createResult`):
    /// `place` holds the inputs, the vanilla result and an initially empty
    /// override. Set `output` to replace the result and `xp` (0 or more)
    /// for what taking it gives; set `cancel` for no result.
    /// Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "GrindstoneEvent.OnPlaceItem",
        cancellable = true
    )]
    fn grindstone_place();
}
