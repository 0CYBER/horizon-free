//! The tail of a mob's death: the wither rose, the copper golem's preserved
//! equipment, and the notice `living_death_post` hears once it has all run.
use super::*;

impl Game<'_> {
    /// `LivingEntity.die` after `dropAllDeathLoot`: the wither rose, the
    /// equipment `dropEquipment` owes the copper golem, and then the
    /// `broadcastEntityEvent` moment (`ServerLivingEntityEvents.AFTER_DEATH`).
    /// `source` is the blow `living_death` heard; a death it cancelled never
    /// reaches here.
    pub(super) fn finish_mob_death(
        &mut self,
        m: &Mob,
        dying: &mut Mob,
        source: Option<(&'static str, u32)>,
    ) {
        self.create_wither_rose(m);
        if m.species() == Species::CopperGolem {
            // CopperGolem.dropEquipment: the preserved equipment too.
            self.golem_drop_equipment(dying);
        }
        if !mod_api::__private::listened(mod_api::HookId::LivingDeathPost) {
            return;
        }
        let (damage_type, attacker) = source.unwrap_or(("minecraft:generic", 0));
        self.native_living_death_post(m.id, damage_type, attacker);
    }
}
