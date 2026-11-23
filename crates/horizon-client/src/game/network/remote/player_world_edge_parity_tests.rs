//! The Rust side of tools/test_player_tick_world_edge_parity.py: with
//! `HORIZON_PLAYER_TICK_TRACE` pointing at a file, one remote player tick from each
//! start position `VanillaPlayerTickOracle` runs leaves the player where vanilla's
//! `Player.tick` leaves it, printed in that oracle's format.
use super::*;

/// `VanillaPlayerTickOracle`'s cases, in its order: inside the edge, past it on each
/// axis, on it, at the clamp, above it, past it on both negative axes, and two ticks
/// that cross it in their own move. Each carries the velocity that move starts with,
/// which is all the tick moves the player by here and in the oracle alike.
const CASES: [(&str, f64, f64, f64, f64, f64, f64); 11] = [
    ("inside_x", 29_999_980.0, 64.0, 8.0, 0.0, 0.0, 0.0),
    ("past_x", 30_000_020.0, 64.0, 8.0, 0.0, 0.0, 0.0),
    ("past_negative_x", -30_000_020.0, 64.0, -8.0, 0.0, 0.0, 0.0),
    ("past_z", 8.0, 64.0, 30_000_020.0, 0.0, 0.0, 0.0),
    ("past_negative_z", 8.0, 64.0, -30_000_020.0, 0.0, 0.0, 0.0),
    ("on_the_edge", 30_000_000.0, 64.0, 8.0, 0.0, 0.0, 0.0),
    ("at_the_clamp", 29_999_999.0, 64.0, 8.0, 0.0, 0.0, 0.0),
    ("high_y", 8.0, 30_000_000.0, 8.0, 0.0, 0.0, 0.0),
    (
        "past_negative_x_and_z",
        -30_000_020.0,
        64.0,
        -30_000_020.0,
        0.0,
        0.0,
        0.0,
    ),
    ("crossing_x", 29_999_980.0, 64.0, 8.0, 40.0, 0.0, 0.0),
    (
        "crossing_negative_z",
        8.0,
        64.0,
        -29_999_980.0,
        0.0,
        0.0,
        -40.0,
    ),
];

/// The fresh spectator `player_world_edge_tests` ticks: flying, its position put there
/// by the fixture rather than by a move, so the tick's clamp is the only thing that
/// pulls it back.
fn spectator_at(game: &mut Game<'_>, at: Vec3, velocity: Vec3) {
    let mut player = PlayerMotion::new(game.motion, at).unwrap();
    player.spectator = true;
    player.flying = true;
    player.velocity = velocity;
    game.player = Some(player);
}

#[test]
#[ignore = "parity harness: tools/test_player_tick_world_edge_parity.py runs it"]
fn a_tick_from_the_world_edge_lands_like_vanilla() {
    let Some(trace) = std::env::var_os("HORIZON_PLAYER_TICK_TRACE") else {
        return;
    };
    with_client_game(|game| {
        let server = connect(game);
        let mut rows = Vec::new();
        for &(name, x, y, z, vx, vy, vz) in &CASES {
            let at = Vec3::new(x, y, z);
            light_column(game, at);
            spectator_at(game, at, Vec3::new(vx, vy, vz));
            game.tick_remote_player(true, false).unwrap();
            let end = game.player.as_ref().unwrap().position;
            rows.push(format!(
                "{{\"case\":\"{name}\",\"x\":\"{:016x}\",\"y\":\"{:016x}\",\"z\":\"{:016x}\"}}",
                end.x.to_bits(),
                end.y.to_bits(),
                end.z.to_bits()
            ));
        }
        server.shutdown();
        std::fs::write(&trace, format!("[{}]\n", rows.join(","))).expect("the trace is written");
    });
}
