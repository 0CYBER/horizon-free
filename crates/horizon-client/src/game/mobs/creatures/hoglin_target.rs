//! Apply a hoglin broadcast after its vanilla target selection and mod approval.
use super::{Game, Vec3, Who};

impl Game<'_> {
    pub(super) fn hoglin_attack_target(&mut self, mob: u32, target: Who, player: Option<u32>) {
        let places: Vec<(u32, Vec3)> = self
            .mobs
            .iter()
            .filter(|m| m.alive())
            .map(|m| (m.id, m.position))
            .collect();
        let current_player = self
            .lan
            .as_ref()
            .and_then(|host| host.mob_players.get(&mob).copied());
        let position = |who: Who, current: bool| match who {
            Who::Mob(id) => places.iter().find(|(i, _)| *i == id).map(|(_, p)| *p),
            Who::Player => {
                let connection = if current { current_player } else { player };
                let id = u32::try_from(self.player_entity_id_by_connection(connection)?).ok()?;
                let epoch = self.native_lifecycle.epoch()?;
                let p = self.entity_position(mod_api::EntityRef::new(epoch, id))?;
                Some(Vec3::new(p[0], p[1], p[2]))
            }
        };
        let proposal =
            self.mobs.iter().find(|m| m.id == mob).and_then(|m| {
                horizon_client::mobs::piglin::receive_attack_target(m, target, position)
            });
        if let Some(target) = proposal {
            self.set_event_ai_target(mob, Some(target), player);
        }
    }
}
