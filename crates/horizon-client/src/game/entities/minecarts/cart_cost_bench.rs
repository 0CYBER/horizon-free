//! What `tick_carts` costs with many parked carts and some moving ones (an ignored measurement).
use super::{CartKind, Minecart};
use crate::game::game_tests::tests::server_game;
use horizon_core::geometry::Vec3;

/// Times `tick_carts` with many parked carts and some moving ones:
/// `CARTS_PARKED` (default 600) carts each on its own rail piece, and
/// `CARTS_MOVING` (default 100) on powered tracks. Prints the ms per tick.
#[test]
#[ignore = "a timing, needs prepared resources in HORIZON_TEST_RESOURCES"]
fn tick_carts_with_many_parked_carts() {
    let n = |key: &str, default: usize| {
        std::env::var(key)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    };
    let (parked, moving) = (n("CARTS_PARKED", 600), n("CARTS_MOVING", 100));
    let mut resources = None;
    let mut game = server_game(&mut resources);
    game.settings.simulation_distance = 6;
    game.settings.render_distance = 12;
    game.player = Some(
        horizon_client::motion::PlayerMotion::new(game.motion, Vec3::new(8.5, 80.0, 12.5)).unwrap(),
    );
    for cx in -3..=3 {
        for cz in -3..=3 {
            if (cx, cz) != (0, 0) {
                let mut chunk = horizon_client::chunk::Chunk::empty(cx, cz);
                chunk.lit = true;
                game.world
                    .chunks
                    .insert((cx, cz), std::sync::Arc::new(chunk));
                game.block_ticks
                    .add_empty_chunk(horizon_core::position::ChunkPos::new(cx, cz))
                    .unwrap();
            }
        }
    }
    game.update_chunk_tickets();
    let registry = game.pack.assets.registry;
    let stone = registry.default_state("minecraft:stone").unwrap();
    let rail = registry.default_state("minecraft:rail").unwrap();
    let powered = registry.default_state("minecraft:powered_rail").unwrap();
    let with = |base: horizon_world::registry::BlockStateId, shape: &str, on: bool| {
        let mut s = registry.state(base).unwrap().clone();
        s.properties.insert("shape".into(), shape.into());
        if on {
            s.properties.insert("powered".into(), "true".into());
        }
        registry.id(&s).unwrap()
    };
    let (rail_ns, rail_ew) = (
        with(rail, "north_south", false),
        with(powered, "east_west", true),
    );
    for x in -30..46 {
        for z in -30..46 {
            game.world.set(x, 63, z, stone).unwrap();
            game.world.set(x, 69, z, stone).unwrap();
        }
    }
    let mut id = 1;
    'park: for x in (-28..44).step_by(2) {
        for z in (-28..44).step_by(2) {
            if id as usize > parked {
                break 'park;
            }
            game.world.set(x, 64, z, rail_ns).unwrap();
            let at = Vec3::new(f64::from(x) + 0.5, 64.0, f64::from(z) + 0.5);
            game.carts.push(Minecart::new(id, CartKind::Rideable, at));
            id += 1;
        }
    }
    for lane in 0..10 {
        let z = -28 + lane * 6;
        for x in -30..46 {
            game.world.set(x, 70, z, rail_ew).unwrap();
        }
        for k in 0..moving.div_ceil(10) {
            if id as usize > parked + moving {
                break;
            }
            let at = Vec3::new(-29.5 + f64::from(k as i32) * 3.0, 70.0, f64::from(z) + 0.5);
            let mut c = Minecart::new(id, CartKind::Rideable, at);
            c.velocity = Vec3::new(0.3, 0.0, 0.0);
            game.carts.push(c);
            id += 1;
        }
    }
    let chicken = game
        .res
        .mob_types()
        .get("minecraft:chicken")
        .unwrap()
        .clone();
    for k in 0..n("MOBS", 3000) {
        let (x, z) = (
            -30.0 + (k % 76) as f64 + 0.5,
            -30.0 + ((k / 76) % 76) as f64 + 0.5,
        );
        game.mobs.push(horizon_client::mobs::Mob::new(
            1000 + k as u32,
            chicken.clone(),
            Vec3::new(x, 75.0, z),
            0.0,
        ));
    }
    let total = game.carts.len();
    let (warm, ticks) = (5, n("TICKS", 40));
    let mut spent = std::time::Duration::ZERO;
    for t in 0..warm + ticks {
        game.world.game_time += 1;
        // Mobs move every tick on a live server: the list is written.
        if !game.mobs.is_empty() {
            let _ = &mut game.mobs[0];
        }
        let start = std::time::Instant::now();
        game.tick_carts();
        if t >= warm {
            spent += start.elapsed();
        }
    }
    eprintln!(
        "tick_carts: {total} carts ({parked} parked, {moving} moving): {:.3} ms/tick",
        spent.as_secs_f64() * 1e3 / ticks as f64
    );
}
