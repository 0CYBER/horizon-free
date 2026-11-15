//! The `damage_item` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `IItemExtension.damageItem`: the wear `stack` is about to take
    /// (`ItemStack.hurtAndBreak`: a tool that mines or hits, armour that takes
    /// a hit, a shield, a rod, an elytra, a tool a dispenser uses, a mob's
    /// armour), after Unbreaking has had its say, as NeoForge passes it.
    /// `entity` is who wears or uses it: the acting player on a player's
    /// paths, the mob, armour stand or dragon wearing it, `None` for a
    /// dispenser's tool. `amount` is the points about to be lost; change it
    /// (0 or less leaves the item whole) or set `canceled` for no wear at
    /// all. Set `break_now` and the item breaks whatever durability it has
    /// left, through the vanilla break (the sound and particles, the stack
    /// emptied, `player_destroy_item` for a player): NeoForge's `onBroken`.
    /// It wins over `canceled`. Not asked when
    /// Unbreaking already saved the item, nor for a creative player's tools
    /// (they take no wear). Only the `ItemPart`s attached to `stack`'s item
    /// hear it: an item with none costs one table lookup, and a game with no
    /// such part none at all. On the paths of a player the player's state is
    /// swapped in, so `host.players().acting_player()` answers them. Once for
    /// each wear, on a server.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "canceled",
        scope = "item",
        key = "stack",
        version = 2,
        neoforge = "IItemExtension.damageItem"
    )]
    fn damage_item();
}
