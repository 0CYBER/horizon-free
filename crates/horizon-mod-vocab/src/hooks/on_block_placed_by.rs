//! The `on_block_placed_by` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `BlockItemExtension.onBlockPlacedBy`: this item's block has been placed,
    /// its final block state and stack block-entity data applied, and
    /// `Block.setPlacedBy` completed (including companion blocks). Runs once,
    /// at the primary position, before sounds, level events and stack consumption.
    /// `stack` is the pre-shrink stack; `placer` is the placing player, or None
    /// for a dispenser. `face` and `click` are the placement context's clicked
    /// face and world-space hit point. Failed, refused and canceled placements
    /// never notify. Only Parts attached to the stack's item hear this server notice.
    #[notes = r##"### After a block item places its block

`ItemPart::on_block_placed_by` is the server-side `BlockItemExtension.onBlockPlacedBy`
notice (hook version 1). It runs once after the final state, stack block-entity data
and `Block.setPlacedBy` work exist, including a bed's head, at the primary position
(the bed's foot). The event carries the pre-shrink `stack`, optional `placer`, `pos`,
final `state`, clicked `face` and world-space `click`. A dispenser has no placer.
Use the `ServerHost` and block-entity handles to copy stack data or link companions;
writes are not overwritten by the placement's stack-data application. The notice
precedes level events and consumption and cannot cancel the placement. A refusal,
placement failure or `entity_place` cancellation never fires it. Items without a
subscribed Part cost one `item_listens` lookup and no stack conversion.
"##]
    #[hook(
        side = "server",
        contract = "listen",
        scope = "item",
        key = "stack",
        neoforge = "BlockItemExtension.onBlockPlacedBy"
    )]
    fn on_block_placed_by(
        stack: &ItemStack,
        placer: Option<PlayerRef>,
        pos: [i32; 3],
        state: BlockState,
        face: BlockFace,
        click: [f64; 3],
    );
}
