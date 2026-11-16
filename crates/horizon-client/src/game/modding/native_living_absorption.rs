//! A living entity's absorption amount for native server mods, through a
//! `LivingRef` (docs/modding/src/how-to/native-mods.md, "Entity and level
//! handles"): `LivingEntity.getAbsorptionAmount` and `setAbsorptionAmount`.
//! The read answers on both sides for the local player; the write is
//! server only, journaled like `living_set_health`.
use super::Game;
use super::commands::Target;
use mod_api::LivingRef;

impl Game<'_> {
    /// The entity's absorption amount (`LivingEntity.getAbsorptionAmount`).
    /// During a player's turn only that player resolves, as for health. None
    /// for a stale handle, an armor stand or the dragon (they carry none).
    pub(super) fn living_absorption(&self, living: LivingRef) -> Option<f32> {
        match self.native_living_read_target(living)? {
            Target::Player => Some(self.player.as_ref()?.health.absorption()),
            Target::Guest(conn) => {
                let guest = self.command_guest(conn)?;
                Some(
                    guest
                        .motion
                        .as_ref()
                        .map_or(guest.saved_absorption.unwrap_or(0.0), |m| {
                            m.health.absorption()
                        }),
                )
            }
            Target::Mob(id) => self
                .mobs
                .iter()
                .find(|m| m.id == id && !m.removed)
                .map(|m| m.absorption),
            _ => None,
        }
    }

    /// Sets the entity's absorption amount (`LivingEntity.setAbsorptionAmount`),
    /// cut to `0..=max_absorption`. No heal or damage hook runs. A player's new
    /// amount reaches their client and the trackers through the synced data.
    /// False for a stale handle, a non-finite amount, an armor stand or the
    /// dragon.
    pub(super) fn living_set_absorption(&mut self, living: LivingRef, amount: f32) -> bool {
        if !amount.is_finite() {
            return false;
        }
        match self.native_living_target(living) {
            Some(t @ (Target::Player | Target::Guest(_))) => self
                .for_player(t, |g| {
                    g.player.as_ref()?;
                    g.journal_player_health();
                    g.player.as_mut()?.set_health(amount, true).ok()
                })
                .flatten()
                .is_some(),
            Some(Target::Mob(id)) => {
                self.journal_mob_health(id);
                let Some(m) = self.mobs.iter_mut().find(|m| m.id == id && !m.removed) else {
                    return false;
                };
                let max = horizon_client::mobs::armor::max_absorption(m);
                m.absorption = if amount < 0.0 {
                    0.0
                } else {
                    amount.min(max.max(0.0))
                };
                m.needs_sync = true;
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::native_entity_refs::tests::{bind_local_player, with_game};
    use horizon_client::mobs::Mob;
    use horizon_client::motion::PlayerMotion;
    use horizon_core::geometry::Vec3;
    use std::sync::Arc;

    /// A zombie whose `minecraft:max_absorption` is 4.
    fn absorbing_zombie(game: &super::Game<'_>) -> Mob {
        let mut kind = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        let kind_mut = Arc::make_mut(&mut kind);
        Arc::make_mut(&mut kind_mut.attributes).insert("minecraft:max_absorption".into(), 4.0);
        Mob::new(12, kind, Vec3::ZERO, 0.0)
    }

    #[test]
    #[ignore = "daily"]
    fn a_players_absorption_is_cut_to_zero_and_max_absorption_and_synced() {
        with_game(|game| {
            let mut player = PlayerMotion::new(game.motion, Vec3::ZERO).unwrap();
            player
                .set_modifier(
                    "minecraft:max_absorption",
                    "test:absorption",
                    10.0,
                    "add_value",
                    true,
                )
                .unwrap();
            game.player = Some(player);
            bind_local_player(game, 20, 123);
            let living = game.living(game.entity(20).unwrap()).unwrap();
            assert_eq!(living.absorption(game), Some(0.0));
            let events = game.player.as_ref().unwrap().events.len();
            assert!(living.set_absorption(game, 8.0));
            assert_eq!(living.absorption(game), Some(8.0));
            assert_eq!(game.player.as_ref().unwrap().health.absorption(), 8.0);
            assert!(
                game.player.as_ref().unwrap().events.len() > events,
                "the new amount is queued for the client and the trackers"
            );
            assert!(living.set_absorption(game, 20.0));
            assert_eq!(living.absorption(game), Some(10.0));
            assert!(living.set_absorption(game, -1.0));
            assert_eq!(living.absorption(game), Some(0.0));
            assert!(living.set_absorption(game, 6.0));
            assert!(!living.set_absorption(game, f32::NAN));
            assert!(!living.set_absorption(game, f32::INFINITY));
            assert_eq!(living.absorption(game), Some(6.0));
            assert_eq!(game.player.as_ref().unwrap().health.absorption(), 6.0);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_mobs_absorption_takes_a_hit_before_its_health() {
        with_game(|game| {
            let zombie = absorbing_zombie(game);
            game.mobs.push(zombie);
            let living = game.living(game.entity(12).unwrap()).unwrap();
            let full = living.health(game).unwrap();
            assert!(living.set_absorption(game, 9.0));
            assert_eq!(living.absorption(game), Some(4.0));
            assert!(living.set_absorption(game, -2.0));
            assert_eq!(living.absorption(game), Some(0.0));
            assert!(!living.set_absorption(game, f32::NAN));
            assert_eq!(living.absorption(game), Some(0.0));
            assert!(living.set_absorption(game, 4.0));
            // Magic damage skips armor: the 3 hearts of the hit come out of the 4.
            let victim = game.entity(12).unwrap();
            assert_eq!(
                victim.hurt(game, "minecraft:magic", 3.0, None, None),
                Some(true)
            );
            assert_eq!(living.absorption(game), Some(1.0));
            assert_eq!(living.health(game), Some(full));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_mobs_set_absorption_survives_a_save_and_reload() {
        with_game(|game| {
            let zombie = absorbing_zombie(game);
            game.mobs.push(zombie);
            let living = game.living(game.entity(12).unwrap()).unwrap();
            assert!(living.set_absorption(game, 3.0));
            let saved = horizon_client::entity_nbt::mob_to_nbt(
                &game.mobs[0],
                [1, 0, 0, 1],
                None,
                None,
                |_| None,
            );
            let loaded =
                horizon_client::entity_nbt::mob_from_nbt(&saved, game.res.mob_types(), 13, None)
                    .unwrap();
            assert_eq!(loaded.absorption, 3.0);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn an_aborted_transaction_restores_the_absorption_of_a_player_and_a_mob() {
        with_game(|game| {
            let mut player = PlayerMotion::new(game.motion, Vec3::ZERO).unwrap();
            player
                .set_modifier(
                    "minecraft:max_absorption",
                    "test:absorption",
                    10.0,
                    "add_value",
                    true,
                )
                .unwrap();
            game.player = Some(player);
            bind_local_player(game, 20, 123);
            let zombie = absorbing_zombie(game);
            game.mobs.push(zombie);
            game.mobs[0].needs_sync = false;
            let player = game.living(game.entity(20).unwrap()).unwrap();
            let mob = game.living(game.entity(12).unwrap()).unwrap();
            assert!(player.set_absorption(game, 5.0));
            assert!(mob.set_absorption(game, 2.0));
            let events = game.player.as_ref().unwrap().events.len();
            game.mobs[0].needs_sync = false;
            let abort = game.transaction_open();
            assert!(player.set_absorption(game, 9.0));
            assert!(mob.set_absorption(game, 4.0));
            game.transaction_close(&abort, false);
            assert_eq!(player.absorption(game), Some(5.0));
            assert_eq!(mob.absorption(game), Some(2.0));
            assert_eq!(game.player.as_ref().unwrap().events.len(), events);
            assert!(!game.mobs[0].needs_sync);
            let commit = game.transaction_open();
            assert!(player.set_absorption(game, 9.0));
            assert!(mob.set_absorption(game, 4.0));
            game.transaction_close(&commit, true);
            assert_eq!(player.absorption(game), Some(9.0));
            assert_eq!(mob.absorption(game), Some(4.0));
            assert!(game.mobs[0].needs_sync);
        });
    }
}
