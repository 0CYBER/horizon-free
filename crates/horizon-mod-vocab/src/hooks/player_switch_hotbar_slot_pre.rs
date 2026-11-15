//! The `player_switch_hotbar_slot_pre` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0291

pub trait Hooks {
    /// `PlayerSwitchHotbarSlotEvent.Pre`: a server player is switching from hotbar slot
    /// `old_slot` to `new_slot` (0 to 8), after the slot range check and before the
    /// selected slot changes. Fired only when the slot differs. The player's state is not
    /// swapped in. Set `cancel` to keep the old slot; the client is sent that slot again,
    /// and `player_switch_hotbar_slot_post` doesn't fire.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "PlayerSwitchHotbarSlotEvent.Pre",
        paper = "PlayerItemHeldEvent",
        cancellable = true
    )]
    fn player_switch_hotbar_slot_pre();
}
