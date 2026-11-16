//! A player's collision exclusions for mods (arismoko/horizon#3717): boxes of its own level in which that level's
//! blocks don't stop it, as a portal in a wall needs. The server keeps them on the guest, and its
//! body takes them at each turn; a client keeps its own player's on the body it moves. Neither
//! side sends its set to the other: a mod sets both.
use super::{Game, commands::Target};
use horizon_world::collision_exclusions::Refused;
use mod_api::{CollisionExclusionsError, EntityRef};

fn refused(why: Refused) -> CollisionExclusionsError {
    match why {
        Refused::TooMany => CollisionExclusionsError::TooMany,
        Refused::NotFinite => CollisionExclusionsError::NotFinite,
        Refused::Inverted => CollisionExclusionsError::Inverted,
    }
}

impl Game<'_> {
    /// For a server mod: the player's collision exclusions, in place of the last ones (the
    /// guest's, which its moves are checked against, and its body's). Refused for a stale handle,
    /// an entity that isn't a player, and boxes the set refuses; the set there was stays.
    pub(super) fn entity_set_collision_exclusions(
        &mut self,
        entity: EntityRef,
        boxes: &[[f64; 6]],
    ) -> Result<(), CollisionExclusionsError> {
        let conn = match self
            .native_entity_target(entity)
            .ok_or(CollisionExclusionsError::Stale)?
        {
            Target::Guest(conn) => conn,
            Target::Player => self.lan_acting.ok_or(CollisionExclusionsError::Stale)?,
            _ => return Err(CollisionExclusionsError::NotAPlayer),
        };
        // During the guest's own turn its body is the game's player.
        let acting = self.lan_acting == Some(conn);
        let host = self.lan.as_mut().ok_or(CollisionExclusionsError::Stale)?;
        let turn_body = host
            .hub
            .update_guest(conn, |g| {
                g.collision_exclusions.set(boxes)?;
                if let Some(body) = g.motion.as_deref_mut() {
                    body.collision_exclusions
                        .clone_from(&g.collision_exclusions);
                }
                Ok::<_, Refused>(acting.then(|| g.collision_exclusions.clone()))
            })
            .ok_or(CollisionExclusionsError::Stale)?
            .map_err(refused)?;
        if let (Some(set), Some(body)) = (turn_body, self.player.as_mut()) {
            body.collision_exclusions = set;
        }
        Ok(())
    }

    /// For a client mod: the local player's collision exclusions, in place of the last ones.
    /// Refused with no player of its own, and for boxes the set refuses; the set there was stays.
    pub(super) fn set_collision_exclusions(
        &mut self,
        boxes: &[[f64; 6]],
    ) -> Result<(), CollisionExclusionsError> {
        let body = self
            .player
            .as_mut()
            .ok_or(CollisionExclusionsError::Stale)?;
        body.collision_exclusions.set(boxes).map_err(refused)
    }
}

#[cfg(test)]
mod tests {
    use crate::game::Game;
    use mod_api::CollisionExclusionsError as Refusal;

    /// A portal's opening, 1 wide and 2 tall.
    const OPENING: [[f64; 6]; 1] = [[2.0, 64.0, 5.0, 3.0, 66.0, 6.0]];
    const TOO_MANY: [[f64; 6]; 65] = [[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]; 65];

    fn guest_boxes(game: &Game<'_>, conn: u32) -> usize {
        let host = game.lan.as_ref().unwrap();
        host.hub
            .guest(conn)
            .unwrap()
            .collision_exclusions
            .boxes()
            .len()
    }

    #[test]
    #[ignore = "daily"]
    fn a_server_mod_sets_a_guests_exclusions_and_a_refused_set_keeps_them() {
        crate::game::native_guest_turn_tests::with_guests(|game| {
            let entity = game.player(12).unwrap().entity();
            assert_eq!(entity.set_collision_exclusions(game, &OPENING), Ok(()));
            assert_eq!(guest_boxes(game, 12), 1);
            assert_eq!(guest_boxes(game, 13), 0, "only that player's");
            let nan = [[0.0, 64.0, 0.0, 1.0, f64::NAN, 1.0]];
            let inverted = [[3.0, 64.0, 5.0, 2.0, 66.0, 6.0]];
            for (boxes, why) in [
                (&TOO_MANY[..], Refusal::TooMany),
                (&nan[..], Refusal::NotFinite),
                (&inverted[..], Refusal::Inverted),
            ] {
                assert_eq!(entity.set_collision_exclusions(game, boxes), Err(why));
                assert_eq!(guest_boxes(game, 12), 1, "{why:?} keeps the opening");
            }
            // The guest's body takes them for its turn, and a set made during the turn is the
            // body's at once.
            let in_turn = game.as_guest(12, |game| {
                let before = game
                    .player
                    .as_ref()
                    .unwrap()
                    .collision_exclusions
                    .boxes()
                    .len();
                assert_eq!(entity.set_collision_exclusions(game, &[]), Ok(()));
                let after = game
                    .player
                    .as_ref()
                    .unwrap()
                    .collision_exclusions
                    .boxes()
                    .len();
                (before, after)
            });
            assert_eq!(in_turn, Some((1, 0)));
            assert_eq!(guest_boxes(game, 12), 0);
            // A guest that left is gone with its boxes: its handle names nothing.
            assert_eq!(entity.set_collision_exclusions(game, &OPENING), Ok(()));
            let i = game.guest_index(12).unwrap();
            game.lan.as_mut().unwrap().hub.guests.remove(i);
            assert_eq!(
                entity.set_collision_exclusions(game, &OPENING),
                Err(Refusal::Stale)
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_client_sets_its_own_players_and_a_respawn_or_another_level_clears_them() {
        use horizon_protocol::packets::play::{self, Clientbound as C};
        let mut resources = None;
        let mut game = crate::game::game_tests::tests::server_game(&mut resources);
        game.options.execution_role = crate::game::execution_role::ExecutionRole::Client;
        let (pipe, _server) = horizon_protocol::connection::Pipe::pair();
        game.connect_local(pipe, "Steve").unwrap();
        let spawn = |dimension: &str| play::SpawnInfo {
            dimension_type: 0,
            dimension: dimension.into(),
            seed: 1,
            game_type: 0,
            previous_game_type: None,
            debug: false,
            flat: false,
            last_death: None,
            portal_cooldown: 0,
            sea_level: 63,
        };
        game.handle_packet(C::Login(play::Login {
            player_id: 7,
            hardcore: false,
            levels: vec!["minecraft:overworld".into()],
            max_players: 20,
            chunk_radius: 8,
            simulation_distance: 8,
            reduced_debug_info: false,
            show_death_screen: true,
            limited_crafting: false,
            spawn: spawn("minecraft:overworld"),
            online_mode: false,
            enforces_secure_chat: false,
        }))
        .unwrap();
        let position = |teleport| {
            C::PlayerPosition(play::PlayerPosition {
                teleport,
                position: [0.5, 65.0, 0.5],
                delta: [0.0; 3],
                yaw: 0.0,
                pitch: 0.0,
                relatives: 0,
            })
        };
        game.handle_packet(position(1)).unwrap();
        let held = |game: &Game<'_>| {
            let player = game.player.as_ref().unwrap();
            player.collision_exclusions.boxes().len()
        };
        assert_eq!(game.set_collision_exclusions(&OPENING), Ok(()));
        assert_eq!(
            game.set_collision_exclusions(&TOO_MANY),
            Err(Refusal::TooMany)
        );
        assert_eq!(held(&game), 1, "the refused set keeps the opening");
        // A death's Respawn, in the same level.
        game.handle_packet(C::Respawn(play::Respawn {
            spawn: spawn("minecraft:overworld"),
            keep: 0,
        }))
        .unwrap();
        game.handle_packet(position(2)).unwrap();
        assert_eq!(held(&game), 0, "a respawn clears them");
        // A portal's Respawn into another level, keeping all data.
        assert_eq!(game.set_collision_exclusions(&OPENING), Ok(()));
        game.handle_packet(C::Respawn(play::Respawn {
            spawn: spawn("minecraft:the_nether"),
            keep: 3,
        }))
        .unwrap();
        game.handle_packet(position(3)).unwrap();
        assert_eq!(held(&game), 0, "another level clears them");
    }
}
