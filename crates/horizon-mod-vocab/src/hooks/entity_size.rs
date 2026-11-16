//! The `entity_size` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// An entity's size was recalculated (NeoForge's `EntityEvent.Size`, vanilla's
    /// `Entity.refreshDimensions`): its pose changed to `pose`, it grew up, its
    /// `minecraft:scale` changed, or it is new. `size` (width, height and eye height, in
    /// blocks) starts as vanilla's new size and `old_size` is the size it had. Change `size`
    /// and the entity's box and eyes take it; later mods see earlier mods' edits. The answer
    /// holds while the entity keeps that pose and vanilla size; the next change asks again,
    /// starting from vanilla's. Changing the width or height leaves the eye height as it is:
    /// set it too. Zero-size answers are valid. An answer is ignored if any component is
    /// non-finite, negative or greater than 32 blocks. This bounds collision and pathfinding
    /// work while allowing sizes larger than naturally occurring vanilla entities. `host`
    /// reads the side that asks; the entity may be new, so read only what you need of it.
    ///
    /// Asked on the server for its mobs (before and after their ticks) and for each LAN
    /// guest's player (once a tick), and on a client for its own player (after its tick):
    /// a size changed within a tick takes the answer at the next of those. Does not fire for:
    /// other kinds of entity, whose vanilla sizes are fixed, and a client's copies of other
    /// entities, which keep vanilla's size. Not cancellable.
    #[hook(
        side = "both",
        contract = "collect",
        out = "size",
        host = "read",
        neoforge = "EntityEvent.Size",
        cancellable = false
    )]
    fn entity_size();
}
