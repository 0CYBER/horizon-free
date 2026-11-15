//! The `living_equipment_change` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0157

pub trait Hooks {
    /// A mob's, an armor stand's or a player's equipment slot took a different
    /// stack (NeoForge's `LivingEquipmentChangeEvent`, posted in
    /// `LivingEntity.collectEquipmentChanges`): `entity` is its ID and `kind`
    /// its type (`minecraft:zombie`, `minecraft:player`), `slot` the
    /// `EquipmentSlot` (`mainhand`, `offhand`, `feet`, `legs`, `chest`, `head`,
    /// `body`, `saddle`), `from` what the slot held when this was last heard
    /// (none for empty) and `to` what it holds now. Once for each slot that
    /// changed, in that order, after the mobs' tick, so a mob that joins the
    /// level wearing something counts from empty on its first tick. It only
    /// hears: change the entity with its own helpers.
    ///
    /// Players are told the same way, once a tick, for the players connected
    /// to the server (a dedicated server's, a published world's guests): a
    /// player new to the level counts from empty, and the main hand is the
    /// selected hotbar slot, so a hotbar selection or a pickup into the
    /// selected slot is a `mainhand` change. Only their six slots `mainhand`,
    /// `offhand`, `feet`, `legs`, `chest` and `head` are told: a player's
    /// `body` and `saddle` slots are never told. Mobs and players are told
    /// only while a mod sets `living_equipment_listened`. The world
    /// host's own player, in single player or hosting a LAN world, isn't told
    /// here: `on_equip` hears it. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "LivingEquipmentChangeEvent",
        cancellable = false
    )]
    fn living_equipment_change();
}
