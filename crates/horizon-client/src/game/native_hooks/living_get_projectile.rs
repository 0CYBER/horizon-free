use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// `LivingEntity.getProjectile(weapon)` (NeoForge's
    /// `LivingGetProjectileEvent`) for a player (`player` and `name`) or a
    /// mob (`entity`, `kind`): a skeleton or illusioner's shot, or the crossbow
    /// load of a pillager or piglin. `projectile` is the vanilla
    /// ammunition; replace it to choose another. A replaced stack is not taken
    /// from the inventory. 376.
    pub(in crate::game) fn native_living_get_projectile(
        &mut self,
        player: Option<u128>,
        name: &str,
        entity: Option<u32>,
        kind: &str,
        weapon: &ItemStack,
        projectile: &mut Option<ItemStack>,
    ) {
        self.fire_living_get_projectile(player, name, entity, kind, weapon, projectile);
    }
}
