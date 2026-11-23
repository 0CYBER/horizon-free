//! `Player.tick`'s world edge at the end of the local player's own tick (#17227).
use super::*;
use horizon_protocol::packets::play::{self, Clientbound as C};

/// One player tick from `at` moving with `velocity`, through the entry a client of a
/// server ticks its own player with; reads where it left the player, so a test can tell
/// a clamp that runs after the move from one that runs before it.
fn tick_from(game: &mut Game<'_>, at: Vec3, velocity: Vec3) -> Vec3 {
    light_column(game, at);
    {
        let player = game.player.as_mut().unwrap();
        player.position = at;
        player.velocity = velocity;
    }
    game.tick_remote_player(true, false).unwrap();
    game.player.as_ref().unwrap().position
}

fn spectator(game: &mut Game<'_>) {
    // Player.tick derives noPhysics from the game mode, so no collision pulls
    // the move back and only the tick's own clamp does.
    let mut player = PlayerMotion::new(game.motion, Vec3::new(8.0, 64.0, 8.0)).unwrap();
    player.spectator = true;
    player.flying = true;
    player.ground.on_ground = true;
    game.player = Some(player);
}

#[test]
#[ignore = "daily"]
fn a_spectators_move_across_the_world_edge_stops_on_it() {
    with_client_game(|game| {
        let server = connect(game);
        spectator(game);
        // The tick's own move is what the clamp acts on. From 20 blocks inside
        // the edge a slow tick stays inside, and a 40-block tick crosses to
        // 30,000,020 and comes back to the edge. A clamp placed before the
        // motion would let that crossing through and fail the second assert.
        let inside = tick_from(
            game,
            Vec3::new(29_999_980.0, 64.0, 8.0),
            Vec3::new(4.0, 0.0, 0.0),
        );
        assert!(
            inside.x > 29_999_980.0 && inside.x < 30_000_000.0,
            "the tick moved the player, still inside the edge: {inside:?}"
        );
        let crossed = tick_from(
            game,
            Vec3::new(29_999_980.0, 64.0, 8.0),
            Vec3::new(40.0, 0.0, 0.0),
        );
        assert_eq!(
            crossed.x, 29_999_999.0,
            "Player.tick's positive X edge, after the move"
        );
        let crossed_z = tick_from(
            game,
            Vec3::new(8.0, 64.0, -29_999_980.0),
            Vec3::new(0.0, 0.0, -40.0),
        );
        assert_eq!(
            crossed_z.z, -29_999_999.0,
            "Player.tick's negative Z edge, after the move"
        );
        // Y has no clamp: the world is not bounded above.
        let up = tick_from(game, Vec3::new(8.0, 30_000_000.0, 8.0), Vec3::ZERO);
        assert_eq!(up.y, 30_000_000.0, "Y is left alone");
        // A player inside the world keeps the position its move gave it.
        let plain = tick_from(game, Vec3::new(8.0, 64.0, 8.0), Vec3::ZERO);
        assert_eq!(
            plain,
            Vec3::new(8.0, 64.0, 8.0),
            "a position inside the world"
        );
        server.shutdown();
    });
}

#[test]
#[ignore = "daily"]
fn a_mounted_tick_keeps_the_seat_past_the_world_edge() {
    with_client_game(|game| {
        let server = connect(game);
        // A seat half a block past the edge. `Entity.rideTick` runs Player.tick,
        // clamp and all, and then seats the rider on the vehicle
        // (`Entity.java:2433-2439`), so the seat's position is what the tick
        // ends with; a clamp applied after the seating would pull the rider off
        // it every tick.
        let mut player = PlayerMotion::new(game.motion, Vec3::new(8.0, 64.0, 8.0)).unwrap();
        player.ground.on_ground = true;
        game.player = Some(player);
        light_column(game, Vec3::new(8.0, 64.0, 8.0));
        let seat = Vec3::new(29_999_999.5, 64.0, 8.0);
        game.spawn_entity(
            90,
            "minecraft:area_effect_cloud",
            [seat.x, seat.y, seat.z],
            [0.0; 3],
        );
        let me = game.remote.as_ref().unwrap().entity_id;
        game.handle_packet(C::SetPassengers(play::SetPassengers {
            vehicle: 90,
            passengers: vec![me],
        }))
        .unwrap();
        assert_eq!(game.riding, Some(90), "the local player rides the seat");
        game.tick_remote_player(true, false).unwrap();
        let at = game.player.as_ref().unwrap().position;
        assert!(
            (at.x - seat.x).abs() < 1e-6,
            "the rider keeps the seat's x, {at:?} against {seat:?}"
        );
        assert!(
            (at.z - seat.z).abs() < 1e-6,
            "the rider keeps the seat's z, {at:?} against {seat:?}"
        );
        server.shutdown();
    });
}
