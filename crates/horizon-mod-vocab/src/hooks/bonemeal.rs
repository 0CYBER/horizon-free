//! The `bonemeal` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `BonemealEvent`: bone meal from `stack` used on the block `state` at
    /// `pos` (`BoneMealItem.growCrop`), by `player` or, when it is `None`, by
    /// a dispenser or a villager. `valid_bonemeal_target` is the block's
    /// `isValidBonemealTarget`. Cancel skips vanilla's growth, the block's
    /// own bone meal Parts and the meal's use: `growCrop` then answers
    /// `successful` (`setSuccessful`), which is `false` unless a mod sets it.
    /// Fired on both sides; changes to `stack` stay. Set it to None to consume
    /// the whole stack; a nonempty result is validated before any write-back.
    #[hook(
        side = "both",
        contract = "cancel",
        out = "canceled",
        neoforge = "BonemealEvent",
        cancellable = true
    )]
    fn bonemeal(
        dimension: LevelDimension,
        player: Option<u128>,
        player_entity_id: Option<i32>,
        pos: BlockPos,
        state: BlockState,
        source: BonemealSource,
        stack: &mut Option<ItemStack>,
        valid_bonemeal_target: bool,
        successful: &mut bool,
        canceled: &mut bool,
    );
}
