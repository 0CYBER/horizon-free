use super::*;

impl HitLevel<'_> {
    /// NeoForge's `LivingUseTotemEvent`: a killing blow on entity `entity` of type `kind`
    /// from `source` (named as `native_living_incoming_damage` names it) would be taken by the
    /// `death_protection` item `item` in its `hand` (main or off;
    /// `LivingEntity.checkTotemDeathProtection`). Set `cancel` and the entity gets no
    /// protection: it dies and the item is kept. A player or a mob holding a totem fires it; a
    /// source that bypasses invulnerability never reaches it. `native_player_totem` still
    /// hears a server player's totem that did save them, after this. The level is the hit's: its blocks, random and own
    /// attachments.
    pub(in crate::game) fn native_living_use_totem(
        &mut self,
        entity: i32,
        kind: &str,
        source: &HurtSource<'_>,
        hand: Hand,
        item: &str,
        cancel: &mut bool,
    ) {
        self.fire_living_use_totem(entity, kind, source, hand, item, cancel);
    }
}
