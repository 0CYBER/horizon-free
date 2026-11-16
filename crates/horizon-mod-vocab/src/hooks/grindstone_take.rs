//! The `grindstone_take` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The server player `player` (UUID) and `player_entity_id` took a
    /// grindstone's result at `pos` (`GrindstoneEvent.OnTakeItem`, in its
    /// result slot's `onTake`): `take` holds the inputs, the experience (-1
    /// for vanilla's, or what `grindstone_place` set) and what the inputs
    /// hold afterwards (empty). Set `xp` (0 or more) to give that much
    /// experience, `new_top` and `new_bottom` to leave items in the inputs.
    /// Set `cancel` to skip the rest of `onTake`: the result is still taken,
    /// but no experience, no sound, and the inputs stay. Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "GrindstoneEvent.OnTakeItem",
        cancellable = true
    )]
    fn grindstone_take();
}
