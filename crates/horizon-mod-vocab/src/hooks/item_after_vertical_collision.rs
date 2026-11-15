//! Fork-shaped conversion of the upstream declaration.

pub trait Hooks {
    /// `native_item_after_vertical_collision` lends a dropped item's active stack after vertical collision response, before block effects, merging and pickup. The block is the one selected by the legacy on-position query. `client` identifies prediction. Set `discard` (trait `Flow::Cancel`) or set the stack to None to consume the item without loot or pickup. A movement skipped by the normal item tick sends no notification. Bundles need "hooks": @API:item_after_vertical_collision@.
    #[hook(
        side = "both",
        contract = "cancel",
        out = "discard",
        scope = "block",
        key = "state",
        neoforge = "none (Block.updateEntityAfterFallOn; dropped-item contact)"
    )]
    fn item_after_vertical_collision(
        dimension: &str,
        client: bool,
        entity: u32,
        state: BlockState,
        block_pos: [i32; 3],
        item_pos: [f64; 3],
        stack: &mut Option<ItemStack>,
        discard: &mut bool,
    );
}
