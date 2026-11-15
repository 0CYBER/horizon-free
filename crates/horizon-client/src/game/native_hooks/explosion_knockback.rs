use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// An explosion at `center` in `dimension`, about to break `blocks`, pushes
    /// `entity` (None for a player, whom `native_acting_player` names):
    /// `ServerExplosion.hurtEntities`, after the knockback is calculated. Change `knockback_velocity` to change
    /// what is added to the entity's velocity; a player's client is sent the
    /// same. Wind bursts too, except dragons and end crystals. End crystals
    /// are also omitted from ordinary blasts: Horizon removes them without a
    /// velocity to change. Server-side. 374.
    pub(in crate::game) fn native_explosion_knockback(
        &mut self,
        dimension: &str,
        center: [f64; 3],
        blocks: &[[i32; 3]],
        entity: Option<u32>,
        knockback_velocity: &mut [f64; 3],
    ) {
        self.fire_explosion_knockback(dimension, center, blocks, entity, knockback_velocity);
    }
}
