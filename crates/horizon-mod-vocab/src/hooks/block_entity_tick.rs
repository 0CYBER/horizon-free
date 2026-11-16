//! The `block_entity_tick` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0024

pub trait Hooks {
    /// A server tick of the block entity at `pos`, in vanilla's order among
    /// the others (`Level.tickBlockEntities`), when its `id` is yours: read
    /// and change its saved compound `nbt`, its item slots `items` (keep
    /// their count: as many as its container has, none without one) and the
    /// data slots its menu shows (`data`). It is out of the world during the
    /// call (the block entity at `pos` reads as a `Mod` with an empty id, and
    /// a save leaves it out); if the call removes or replaces it, `entity` is
    /// dropped. Clients, the window's own in single player included, get
    /// what `block_entity_update_tag` gives when its chunk is sent or
    /// its block changes.
    /// Each Part starts with `Changed` and sees the preceding accepted payload.
    /// `*event.reply = BlockEntityTickReply::Unchanged` discards only this Part's
    /// payload edits: no reply snapshot is encoded, decoded or applied. Earlier
    /// Parts' `Changed` edits survive later `Unchanged` replies.
    /// If all Parts answer `Unchanged`, ticking alone does not dirty the chunk,
    /// advance its revision or notify containers/comparators. The original
    /// placeholder must still exist; independent Host writes keep their own effects,
    /// including accepted item writes, while aborted Host transactions stay aborted.
    /// The default `Changed` retains full checked write-back.
    #[hook(
        side = "server",
        contract = "listen",
        scope = "block_entity",
        key = "entity",
        neoforge = "none (vanilla BlockEntityTicker; NeoForge adds no event or extension for it)"
    )]
    fn block_entity_tick();
}
