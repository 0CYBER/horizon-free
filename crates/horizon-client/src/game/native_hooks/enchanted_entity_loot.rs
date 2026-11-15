use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// An entity's loot table reads an enchantment's level (NeoForge's
    /// `EnchantedEntityLootEvent`), in the middle of the roll. `level` comes
    /// in as the living attacker's level, 0 without one, and what is left in
    /// it is what the table uses.
    pub(in crate::game) fn native_enchanted_entity_loot(
        &mut self,
        entity: LivingRef,
        source: &HurtSource<'_>,
        attacker: Option<EntityRef>,
        enchantment: &Identifier,
        level: &mut u32,
    ) {
        self.fire_enchanted_entity_loot(entity, source, attacker, enchantment, level);
    }
}
