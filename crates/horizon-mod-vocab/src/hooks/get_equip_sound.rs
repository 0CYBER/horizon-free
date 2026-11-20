//! The `get_equip_sound` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `IItemExtension.getEquipSound`: the sound event `stack`
    /// plays when it is equipped in equipment slot `slot` (`head`, `chest`,
    /// `legs`, `feet`, `offhand`, `mainhand`, or a mod's own place name such as
    /// `curios:ring`). `sound` arrives with the stack's own
    /// `Equippable.equipSound`, `None` for the generic armour sound; a part
    /// attached to `stack`'s item may change it to any `namespace:path` id,
    /// including one from its own content mod's `assets/<id>/sounds.json`
    /// (Horizon sends a mod's sound to players by name). `None` keeps the
    /// generic armour sound. The slot name lets one item answer differently
    /// for different places. Asked on the server wherever the game plays an
    /// equip sound: a player's armour and off hand, a mob's or an armor
    /// stand's equipment through a `LivingRef`, a mount's saddle or body
    /// armour, and a dispenser's armour. The sound is chosen before it is
    /// played; changing other state here is not a safe way to cancel the
    /// equip itself. Only the `ItemPart`s attached to `stack`'s item are asked.
    /// Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        scope = "item",
        key = "stack",
        host = "read",
        neoforge = "IItemExtension.getEquipSound"
    )]
    fn get_equip_sound();
}
