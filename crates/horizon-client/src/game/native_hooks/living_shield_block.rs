use super::*;

impl HitLevel<'_> {
    /// NeoForge's `LivingShieldBlockEvent`: entity `entity` of type `kind`
    /// (`minecraft:player`, `minecraft:zombie`) is hit for `damage` from `source` (its
    /// type and entities, as `native_living_incoming_damage` names them), and
    /// `LivingEntity.applyItemBlocking` is
    /// about to resolve what a raised `blocks_attacks` item stops. Every hit with damage
    /// asks, raised item or not: `original_blocked` is whether vanilla blocks (only a
    /// player raises one). Change `block`: `blocked` makes the entity block or not (a block
    /// forced on a hit vanilla lets through stops all of it), `blocked_damage` what is
    /// stopped (kept within 0 and `damage`) and `shield_damage` the durability a raised
    /// item loses (below 0: vanilla's `itemDamage` of what is stopped). Set `blocked` to false and nothing is blocked; the event has no cancel. The level is the hit's: its blocks, random and own
    /// attachments.
    pub(in crate::game) fn native_living_shield_block(
        &mut self,
        entity: i32,
        kind: &str,
        source: &HurtSource<'_>,
        damage: f32,
        original_blocked: bool,
        block: &mut ShieldBlock,
    ) {
        self.fire_living_shield_block(entity, kind, source, damage, original_blocked, block);
    }
}
