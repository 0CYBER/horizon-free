use super::*;

impl Game<'_> {
    /// Send one selected attack with the same prediction and charge bookkeeping as vanilla.
    pub(in crate::game) fn remote_attack_entity(&mut self, id: i32, distance: f64) -> bool {
        if let Some(index) = self.mobs.iter().position(|mob| mob.id as i32 == id) {
            if !self.held_range_allows(distance) {
                return false;
            }
            self.send(S::Attack(play::Attack { id }));
            let kind = format!(
                "minecraft:{}",
                self.mobs[index].kind.id.trim_start_matches("minecraft:")
            );
            self.remote_sweep_event(&kind, true);
            self.pvp_attacked(id, distance);
            self.remote_player_attack(index);
        } else {
            self.send(S::Attack(play::Attack { id }));
            if let Some(projectile) = self
                .creatures
                .projectiles
                .iter()
                .find(|p| p.id as i32 == id)
            {
                if !crate::game::deflect::redirectable(projectile) {
                    self.remote_sweep_event("minecraft:shulker_bullet", false);
                }
            } else if let Some((kind, living)) = self.remote_attacked_object(id) {
                self.remote_sweep_event(&kind, living);
            }
        }
        self.reset_attack_strength();
        true
    }
}
