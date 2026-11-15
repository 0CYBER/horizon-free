//! The `is_not_replaceable_by_pick_action` hook: declarations the mod API generator reads (see
//! `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `IItemExtension.isNotReplaceableByPickAction`: must a pick (the pick-block
    /// key) leave `stack`, in hotbar `slot` (0 to 8), where it is? `player` is the player
    /// picking. `not_replaceable` arrives with vanilla's answer, whether the stack is
    /// enchanted; set it `true` to keep the stack in place or `false` to let the pick swap it
    /// out. Asked in `Inventory.getSuitableHotbarSlot`'s second pass, when a pick moves an item
    /// into the hotbar and no hotbar slot is empty: from the selected slot on, for each hotbar
    /// stack, until one answers `false`, which is the slot the pick uses; when all answer
    /// `true`, the selected slot is used. A pick that selects a matching hotbar stack, or
    /// finds an empty slot, doesn't ask, nor one `player_pick_item` cancelled. Only the
    /// `ItemPart`s attached to `stack`'s item are asked, on the server (the host's own player
    /// and its guests), with a read-only `Host`. The contract beyond the method's name and
    /// parameters is Horizon's reading, not checked against NeoForge's.
    #[hook(
        side = "server",
        contract = "listen",
        scope = "item",
        key = "stack",
        host = "read",
        neoforge = "IItemExtension.isNotReplaceableByPickAction"
    )]
    fn is_not_replaceable_by_pick_action();
}
