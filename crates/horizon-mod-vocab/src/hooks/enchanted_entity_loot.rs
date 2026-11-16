//! The `enchanted_entity_loot` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// An entity's loot table reads an enchantment's level (NeoForge's
    /// `EnchantedEntityLootEvent`): asked in the middle of the roll, each
    /// time an `enchanted_count_increase` function or a
    /// `random_chance_with_enchanted_bonus` condition is evaluated, for a
    /// death and for `/loot ... kill`. `entity` is the living entity dropping
    /// the loot, `source` the killing blow (`minecraft:magic` for the command)
    /// and `attacker` the attacking entity (`getAttacker`): the one that
    /// caused the blow, or the command's source, even when `source` names
    /// nobody. It is None without one, and it need not be living: such a read
    /// is asked too, from a level of 0. `level` comes in as the living
    /// attacker's level of `enchantment`; what a mod leaves in it is what the
    /// table uses, so a level raised from 0 counts even with no attacker. A
    /// level can't be negative, and one above 1024 counts as 1024: vanilla's
    /// levels stop at 255, and a table's count grows with the level. Mods that don't need the entities prefer
    /// `get_enchantment_level`; a mob's equipment drop chance isn't this
    /// hook's (`get_equipment_drop_chance`). Not asked for a `/loot ... kill`
    /// of an entity that isn't living, nor for a table rolled any other way.
    /// The draws a mod makes from the level's random stay in order with the
    /// roll's. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "EnchantedEntityLootEvent",
        cancellable = false
    )]
    fn enchanted_entity_loot();
}
