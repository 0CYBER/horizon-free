use super::*;

impl HitLevel<'_> {
    /// NeoForge's `LivingDamageEvent.Pre`: `LivingEntity.actuallyHurt` of entity `entity` of
    /// type `kind` (`minecraft:player`, `minecraft:zombie`) by `source` (named as
    /// `native_living_incoming_damage` names it) has taken armor and magic, and the absorption
    /// hearts are next: `container` holds the damage it started with, what it is now
    /// (`new_damage`, which may be 0) and each reduction so far (armor, enchantments, mob
    /// effects). Change its
    /// `new_damage` and the health takes that; set its post-attack invulnerability ticks and the
    /// entity keeps them instead of vanilla's. A value that is not a number is put back as it
    /// was. It fires for players, mobs and the ender dragon; a wolf whose armor takes the hit and
    /// armor stands do not fire it. The level is the hit's: its blocks, random and own
    /// attachments; the hurt entity is in the middle of its hit.
    pub(in crate::game) fn native_living_damage_pre(
        &mut self,
        entity: i32,
        kind: &str,
        source: &HurtSource<'_>,
        container: &mut mod_api::DamageContainer,
    ) {
        self.fire_living_damage_pre(entity, kind, source, container);
    }
}
