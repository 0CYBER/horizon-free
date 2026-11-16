use crate::game::{Game, api_values::TryIntoApi};
use mod_api::HurtSource;

impl Game<'_> {
    /// A veto belongs to the guest's living body, not its replaceable combat history.
    pub(in crate::game) fn guest_death_canceled(&mut self, gi: usize, conn: u32) -> bool {
        if self
            .lan
            .as_ref()
            .is_some_and(|host| host.hub.guests[gi].death_canceled)
        {
            return true;
        }
        if !mod_api::__private::listened(mod_api::HookId::LivingDeath) {
            return false;
        }
        let (damage_type, causing) = self.guest_damage_source(conn);
        let Some(guest) = self.lan.as_ref().and_then(|host| host.hub.guests.get(gi)) else {
            return false;
        };
        let entity = guest.entity_id as u32;
        let accepted = guest
            .motion
            .as_ref()
            .and_then(|body| body.health.accepted_hit.as_ref())
            .or(guest.carried_hit.as_ref())
            .filter(|hit| {
                damage_type != "minecraft:generic_kill" || hit.source().damage_type == damage_type
            })
            .cloned();
        let fallback = HurtSource {
            damage_type: &damage_type,
            causing: causing as i32,
            direct: causing as i32,
            position: None,
        };
        let source = accepted.as_ref().map_or(fallback, |hit| hit.source());
        let weapon = accepted
            .as_ref()
            .and_then(|hit| hit.weapon.as_ref())
            .and_then(|stack| stack.try_into_api().ok());
        let mut cancel = false;
        self.native_living_death(
            entity,
            "minecraft:player",
            &source,
            weapon.as_ref(),
            &mut cancel,
        );
        if cancel && let Some(host) = self.lan.as_mut() {
            host.hub.guests.update(gi, |guest| {
                guest.death_canceled = guest.health <= 0.0;
                if !guest.death_canceled {
                    guest.death_time = 0;
                }
                if let Some(body) = guest.motion.as_mut() {
                    body.health.dead = false;
                }
            });
        }
        cancel
    }
}
