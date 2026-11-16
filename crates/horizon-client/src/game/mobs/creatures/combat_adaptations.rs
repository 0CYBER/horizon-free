//! Typed adaptations for upstream mob knockback and breeding draws.
use super::{Damage, Game, HurtSource, Who, source_damage_type};
use horizon_client::mobs::Species;
use horizon_core::random::RandomSource;

impl Game<'_> {
    pub(super) fn mob_pushes_player(
        &mut self,
        power: f64,
        x: f64,
        z: f64,
        source: Damage,
        apply_blocking: Option<f32>,
    ) {
        let mut power = power;
        let attack = self.attack_of(match source.attacker {
            Some(Who::Mob(id)) => id as i32,
            _ => 0,
        });
        if let Some(damage) = apply_blocking
            && let Some(blocking) =
                self.player_blocking(source.from, source_damage_type(source), false)
            && let blocked = blocking.blocked(damage)
            && blocked > 0.0
        {
            let attacker = match source.attacker {
                Some(Who::Mob(id)) => Some(id),
                _ => None,
            };
            self.after_block(
                horizon_client::health::BlockOutcome {
                    blocked,
                    fully_blocked: blocked >= damage,
                    ..Default::default()
                },
                (attacker, false),
                HurtSource {
                    damage_type: source_damage_type(source),
                    causing: attacker.map_or(0, |id| id as i32),
                    direct: 0,
                    position: source.from,
                },
                &attack,
            );
            power *= 0.5;
        }
        let source = HurtSource {
            damage_type: source_damage_type(source),
            causing: 0,
            direct: 0,
            position: None,
        };
        self.knock_back_player(power, x, z, source, &attack, 0.0);
    }
    pub(super) fn bred_wolf_voice(&mut self, parent: u32) -> Option<u8> {
        let voices = horizon_client::mobs::finalize::sound_variants(Species::Wolf);
        self.mobs
            .iter_mut()
            .find(|m| m.id == parent)
            .and_then(|m| m.random.next_int_bound(voices.len() as i32).ok())
            .map(|i| i as u8)
    }
}
