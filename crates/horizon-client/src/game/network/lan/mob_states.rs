//! Mob metadata and live attributes, built on the owning server thread.
use super::*;
impl Game<'_> {
    pub(super) fn fill_mob_state(
        &self,
        m: &Mob,
        hub: &Hub,
        player_entity: Option<i32>,
        speeds: &mut super::super::lan_attributes::SpeedByType,
        e: &mut horizon_client::lan::entity::EntityState,
    ) {
        super::super::villager_client::server_meta(
            m,
            hub,
            player_entity,
            self.world.game_time,
            &mut e.meta,
            &mut e.defaults,
            &self.stations().professions,
        );
        // HappyGhast.aiStep: requiresPrecisePosition while on a still timeout.
        e.precise = m.species() == horizon_client::mobs::Species::HappyGhast
            && horizon_client::mobs::happy_ghast::on_still_timeout(m);
        mob_freeze_meta(m, &mut e.meta, &mut e.defaults);
        self.mob_attributes(m, speeds, &mut e.attributes);
    }
}
