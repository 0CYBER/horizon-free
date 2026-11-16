//! The `entity_place` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `BlockEvent.EntityPlaceEvent` (and `EntityMultiPlaceEvent`, which extends it): `placer` set the block `state` at `pos` against `placed_against`, before it is written. `placer` is the player for `BlockItem.place` (before the stack shrinks), or the mob for an enderman putting its carried block down and a snow golem's trail; `None` is reserved for a placement with no entity. `placement` is `Single`, or `Multi` with every block the placement writes (a bed's foot and head, a door's halves, a tall plant), where `state` and `pos` stay the primary block (the foot, the lower half). For players, `placed_against` is the state adjacent to the primary placement position opposite the clicked face, including when the clicked block is replaceable. For mobs it is the state below the placement. Cancel stops the whole placement: no block is written, the stack is kept and an enderman keeps carrying its block. Server-side. Diverges from upstream: replaces the listen-only `place_block`; a mod that stopped placements by claiming `native_use_on` cancels here.
    #[hook(side = "server", contract = "cancel", out = "canceled", neoforge = ["BlockEvent.EntityPlaceEvent", "BlockEvent.EntityMultiPlaceEvent"], cancellable = true)]
    fn entity_place();
}

/// Original upstream notes; the existing fork event above owns this moment.
pub trait UpstreamNotes {
    #[notes = r###"`native_block_place(player, name, stack, pos, state, cancel)`: a guest is about to place `stack` as `state` at `pos`, heard where `BlockItem.place` has found the position and state and checked survival, before anything is set (Bukkit's `BlockPlaceEvent`, NeoForge's `BlockEvent.EntityPlaceEvent`). Set `cancel` and nothing is placed or used; the client sees the blocks and its inventory as they are and `native_place_block` hears nothing. Bundles that use it need `"hooks": @API:block_place@`."###]
    fn block_place();
}
