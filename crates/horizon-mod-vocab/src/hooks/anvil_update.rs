//! The `anvil_update` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// An anvil at `pos` made vanilla's result with a left input, for
    /// `player` (UUID) and `player_entity_id` (`AnvilUpdateEvent`, after
    /// `AnvilMenu.createResult`), whenever an input or the name changed:
    /// `update` holds the inputs (the right one may be empty, for a rename),
    /// the rename text and vanilla's result, and its `output`, `xp_cost`
    /// and `material_cost` start at vanilla's for the mods to change; set
    /// `cancel` for no result. The signed XP cost is preserved; a cost of
    /// zero or less prevents taking the output, not a level refund. Negative
    /// material costs are clamped to zero. Both sides: the server's menu,
    /// and a server's client menu with the local player.
    #[hook(
        side = "both",
        contract = "cancel",
        out = "cancel",
        neoforge = "AnvilUpdateEvent",
        cancellable = true
    )]
    fn anvil_update();
}
