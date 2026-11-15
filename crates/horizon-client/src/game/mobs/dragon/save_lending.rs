//! Save visibility while authoritative rows are lent out.
use super::*;

impl Game<'_> {
    /// `EnderDragon.kill` (`/kill`): `Entity.kill` removes the entity, it
    /// does not hurt it (the dragon flies its death flight when its health
    /// runs out). The fight's dragon ends the fight there and then
    /// (`dragonFight.setDragonKilled`): the exit portal, a gateway and the
    /// first dragon's egg, and no new dragon. No experience: `tickDeath`
    /// drops it, and a killed dragon never gets there.
    pub(in crate::game) fn kill_dragon(&mut self, id: u32) {
        if self.dragon.as_ref().is_some_and(|d| d.id == id) {
            // Only the fight's own dragon (`dragonFight`) ends it.
            let fight = self.is_fight_dragon();
            let _save_lease = horizon_client::join_seq::visibility::PassLease::enter();
            if let Some(d) = self.dragon.extract() {
                // remove, then gameEvent(ENTITY_DIE), before the fight hears of it.
                self.dragon_died_event(d.id, d.position, d.alive());
                self.dragon.retire_extracted(d.id, d.join);
            }
            if fight {
                self.dragon_killed();
            }
        }
    }
}
