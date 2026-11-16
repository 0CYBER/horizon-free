//! arismoko/horizon#3717: a block sharing volume with one of a player's collision exclusions is no wall: the
//! server's `isInWall` doesn't count it, and the client doesn't push its player out of it.
use crate::game::Game;
use horizon_core::geometry::Vec3;
use std::sync::Arc;

/// A player-sized body at (3.5, 64, 5.5): stone in its column (3, 64..=65, 5) and on its west,
/// north and south sides, so east is the one free side.
fn walled(game: &mut Game<'_>) {
    let mut chunk = horizon_client::chunk::Chunk::empty(0, 0);
    chunk.lit = true;
    game.world.chunks.insert((0, 0), Arc::new(chunk));
    let stone = game
        .pack
        .assets
        .registry
        .default_state("minecraft:stone")
        .unwrap();
    for y in 64..=65 {
        for (x, z) in [(3, 5), (2, 5), (3, 4), (3, 6)] {
            game.world.set(x, y, z, stone).unwrap();
        }
    }
}

fn exclude(game: &mut Game<'_>, boxes: &[[f64; 6]]) {
    let hub = &mut game.lan.as_mut().unwrap().hub;
    hub.update_guest(12, |g| g.collision_exclusions.set(boxes))
        .unwrap()
        .unwrap();
}

/// The opening over the body's own column.
const OPENING: [[f64; 6]; 1] = [[3.0, 64.0, 5.0, 4.0, 66.0, 6.0]];

#[test]
#[ignore = "daily"]
fn a_block_in_a_players_exclusions_does_not_suffocate_it() {
    crate::game::native_guest_turn_tests::with_guests(|game| {
        walled(game);
        let at = [3.5, 64.0, 5.5];
        let body = game
            .lan
            .as_ref()
            .unwrap()
            .hub
            .guest(12)
            .unwrap()
            .dimensions();
        assert!(
            game.in_wall(12, at, body),
            "its eyes are in the stone at y 65"
        );
        exclude(game, &[[10.0, 64.0, 10.0, 11.0, 66.0, 11.0]]);
        assert!(
            game.in_wall(12, at, body),
            "a box elsewhere changes nothing"
        );
        exclude(game, &OPENING);
        assert!(!game.in_wall(12, at, body));
    });
}

#[test]
#[ignore = "daily"]
fn a_block_in_a_players_exclusions_does_not_push_it_out() {
    crate::game::native_guest_turn_tests::with_guests(|game| {
        walled(game);
        let mut body =
            horizon_client::motion::PlayerMotion::new(game.motion, Vec3::new(3.5, 64.0, 5.5))
                .unwrap();
        let nudge = |game: &Game<'_>, body: &horizon_client::motion::PlayerMotion| {
            crate::game::closest_space_nudge(body, &game.world, game.res.facts())
        };
        // Every corner is in the stone; east is the nearest free side: 0.1 a tick east.
        assert_eq!(nudge(game, &body), [Some(0.1), None]);
        body.collision_exclusions.set(&OPENING).unwrap();
        assert_eq!(nudge(game, &body), [None, None]);
    });
}
