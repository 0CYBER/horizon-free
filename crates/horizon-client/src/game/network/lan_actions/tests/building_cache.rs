//! The players' boxes the guests' turns of one tick share (`building_cache`):
//! a guest teleported, killed or put in spectator mode in the middle of the
//! tick blocks placement where it is now, as when each turn built the boxes
//! afresh, not at its box from the start of the tick.
use crate::game::lan_actions::{
    HubBodies, bodies_of, building_player, built_bodies, refresh_bodies, shared,
};
use crate::game::*;
use horizon_client::{
    chunk::Chunk,
    lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig},
    motion::PlayerMotion,
};
use horizon_protocol::packets::configuration::ClientInformation;
use std::sync::{Arc, atomic::AtomicI32, mpsc};

type Boxes = Vec<([f64; 3], [f64; 3])>;

fn game<'a>(resources: &'a mut Option<std::sync::Arc<crate::game::Resources>>) -> Game<'a> {
    let root = std::path::PathBuf::from(
        std::env::var_os("HORIZON_TEST_RESOURCES")
            .expect("set HORIZON_TEST_RESOURCES to prepared resources"),
    );
    let settings = Settings::default();
    let loaded = load_resources(
        horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        crate::game::execution_role::ExecutionRole::IntegratedServer,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
    let res = resources.insert(loaded.resources);
    let mut game = Game::new(
        res,
        loaded.pack,
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
            root.clone(),
        )),
        None,
    )
    .unwrap();
    game.build_registries();
    game.set_allow_commands(true);
    for x in 0..2 {
        game.world
            .chunks
            .insert((x, 0), Arc::new(Chunk::empty(x, 0)));
        game.block_ticks
            .add_empty_chunk(horizon_core::position::ChunkPos::new(x, 0))
            .unwrap();
    }
    assert!(game.run_command("fill 0 99 0 31 99 15 stone"));
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(2.5, 100.0, 2.5)).unwrap());
    let net = NetConfig {
        announce: false,
        ..NetConfig::lan(0, "building-cache-test".into(), 3)
    };
    game.lan_host(
        crate::game::lan::LanSettings {
            port: 0,
            guest_commands: true,
            force_game_mode: false,
        },
        net,
        HubConfig {
            default_game_mode: 0,
            guest_commands: true,
            force_game_mode: false,
            hardcore: false,
            view_distance: 2,
            simulation_distance: 2,
            dimension: Dim::Overworld.id().into(),
            allow_flight: false,
            seed: 0,
            max_players: 3,
            integrated: false,
            levels: Default::default(),
            level_seeds: Default::default(),
        },
    )
    .unwrap();
    for (conn, name, x) in [(2, "Mover", 8.5), (3, "Builder", 14.5)] {
        let (tx, _rx) = mpsc::channel();
        let mut guest = Guest::new(
            Joined {
                conn,
                uuid: conn as u128,
                name: name.into(),
                properties: Vec::new(),
                info: ClientInformation::new(2),
                brand: "test".into(),
                sender: PlayerSender::new(tx, Arc::default()),
                latency: Arc::new(AtomicI32::new(0)),
                address: "127.0.0.1:25565".parse().unwrap(),
                expected: false,
            },
            conn as i32,
            [x, 100.0, 8.5],
            0,
        );
        guest.placed = true;
        guest.loaded = true;
        game.lan.as_mut().unwrap().hub.guests.push(guest);
    }
    game
}

/// The other players' boxes in the Builder's turn: what blocks its placing.
fn seen_by_builder(game: &mut Game<'_>) -> Boxes {
    let seen = game.as_guest(3, |g| g.lan_building_boxes.boxes(Dim::Overworld));

    seen.unwrap()
}

/// The Builder's view at the start of a tick, after `command` (and, with
/// `turns`, the guests' turns that follow it in the tick), and the same
/// without the shared boxes.
fn the_mover_changed_mid_tick(command: &str, turns: bool) -> (Boxes, Boxes, Boxes) {
    let mut resources = None;
    let mut game = game(&mut resources);
    crate::game::lan_actions::building_cache(game.lan.as_ref());
    let before = seen_by_builder(&mut game);
    assert!(game.run_command(command), "{command}");
    if turns {
        game.tick_guests_inner();
    }
    let cached = seen_by_builder(&mut game);
    crate::game::lan_actions::building_cache(None);
    let fresh = seen_by_builder(&mut game);
    (before, cached, fresh)
}

fn mover_at(boxes: &Boxes, x: f64) -> bool {
    boxes
        .iter()
        .any(|(min, max)| min[0] <= x && x <= max[0] && min[1] <= 100.5 && 100.5 <= max[1])
}

#[test]
#[ignore = "daily"]
fn a_teleported_guest_blocks_where_it_went_not_where_the_tick_began() {
    let (before, cached, fresh) = the_mover_changed_mid_tick("tp Mover 24.5 100 8.5", false);
    assert!(mover_at(&before, 8.5));
    assert!(!mover_at(&cached, 8.5) && mover_at(&cached, 24.5));
    assert_eq!(cached, fresh);
}

#[test]
#[ignore = "daily"]
fn a_guest_killed_mid_tick_no_longer_blocks() {
    // The kill takes its health; the guest dies in its own turn of the tick.
    let (before, cached, fresh) = the_mover_changed_mid_tick("kill Mover", true);
    assert!(mover_at(&before, 8.5));
    assert!(!mover_at(&cached, 8.5));
    assert_eq!(cached, fresh);
}

#[test]
#[ignore = "daily"]
fn a_guest_put_in_spectator_mid_tick_no_longer_blocks() {
    let (before, cached, fresh) = the_mover_changed_mid_tick("gamemode spectator Mover", false);
    assert!(mover_at(&before, 8.5));
    assert!(!mover_at(&cached, 8.5));
    assert_eq!(cached, fresh);
}

/// A guest list's bodies as bits: what `boxes` and the turns' ancestors read of each.
fn body_bits(bodies: &HubBodies) -> Vec<(Option<u32>, Dim, Option<[u64; 6]>)> {
    bodies
        .iter()
        .map(|cell| {
            let p = cell.get();
            let bits = p.bounds.map(|b| {
                let (lo, hi) = (b.min_position().components(), b.max_position().components());
                [lo[0], lo[1], lo[2], hi[0], hi[1], hi[2]].map(f64::to_bits)
            });
            (p.conn, p.dimension, bits)
        })
        .collect()
}

fn listed_guest(conn: u32) -> Guest {
    let (tx, _rx) = mpsc::channel();
    let mut guest = Guest::new(
        Joined {
            conn,
            uuid: u128::from(conn),
            name: format!("G{conn}"),
            properties: Vec::new(),
            info: ClientInformation::new(2),
            brand: "test".into(),
            sender: PlayerSender::new(tx, Arc::default()),
            latency: Arc::new(AtomicI32::new(0)),
            address: "127.0.0.1:25565".parse().unwrap(),
            expected: false,
        },
        conn as i32,
        [f64::from(conn), 100.0, 0.5],
        0,
    );
    guest.placed = true;
    guest.loaded = true;
    guest
}

/// The bodies kept by write (`built_bodies`, then `refresh_bodies` after each burst of writes)
/// are the bodies built afresh from the guests, to the bit, after every burst of whatever the
/// guest list allows: moves, deaths, modes, poses, dimensions, a change of kind, joins, leaves,
/// swaps, replacements and removals (a guest's turn changing another guest included). The
/// cached bodies must be the ones in use for most bursts, or the test would prove nothing.
#[test]
fn bodies_kept_by_write_equal_bodies_built_afresh() {
    use horizon_client::lan::guest::GuestKind;
    use horizon_client::lan::guest_list::GuestList;
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = move |n: u64| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed % n
    };
    let dimensions = ["minecraft:overworld", "minecraft:the_nether", "minecraft:the_end"];
    let (mut cached_bursts, mut bursts) = (0, 0);
    for round in 0..40 {
        let mut guests = GuestList::default();
        let mut joined = 0;
        for _ in 0..(3 + next(30)) {
            joined += 1;
            guests.push(listed_guest(joined));
        }
        let mut cache = built_bodies(&guests);
        for burst in 0..60 {
            for _ in 0..next(6) {
                let len = guests.len() as u64;
                if len == 0 {
                    joined += 1;
                    guests.push(listed_guest(joined));
                    continue;
                }
                let i = next(len) as usize;
                match next(24) {
                    0..=7 => guests.update(i, |g| {
                        g.position = [next(50) as f64 * 0.37, 90.0 + next(20) as f64, 1.5];
                    }),
                    8 => guests.update(i, |g| g.dead = !g.dead),
                    9 => guests.update(i, |g| g.loaded = !g.loaded),
                    10 => guests.update(i, |g| g.placed = !g.placed),
                    11 => guests.update(i, |g| g.change_game_mode([0, 1, 2, 3][next(4) as usize])),
                    12 => guests.update(i, |g| g.sneaking = !g.sneaking),
                    13 => guests.update(i, |g| g.sleeping = !g.sleeping),
                    14 => guests.update(i, |g| g.fall_flying = !g.fall_flying),
                    15 => guests.update(i, |g| {
                        g.dimension = dimensions[next(3) as usize].into();
                    }),
                    16 => {
                        let conn = guests[i].conn;
                        guests.update_connection(conn, |g| g.position[0] += 1.0);
                    }
                    17 => guests.update_all(|g| g.position[2] += 0.25),
                    18 => {
                        let conn = guests[i].conn;
                        let _ = guests.motion_mut(conn);
                        let _ = guests.inbox_mut(i);
                    }
                    19 if next(4) == 0 => guests.update(i, |g| {
                        g.kind = if g.kind == GuestKind::Fake {
                            GuestKind::Network
                        } else {
                            GuestKind::Fake
                        };
                    }),
                    20 if next(3) == 0 => {
                        joined += 1;
                        guests.push(listed_guest(joined));
                    }
                    21 if next(3) == 0 => drop(guests.remove(i)),
                    22 if len > 1 => guests.swap(i, next(len) as usize),
                    23 if next(4) == 0 => {
                        joined += 1;
                        drop(guests.replace(i, listed_guest(joined)));
                    }
                    _ => {}
                }
            }
            // The turn ends: the kept bodies catch up, then they are compared with fresh ones.
            refresh_bodies(&guests, &cache);
            let kept = bodies_of(&guests, Some(&cache));
            let fresh = shared(guests.simulation_iter().map(building_player));
            assert_eq!(body_bits(&kept), body_bits(&fresh), "round {round}, burst {burst}");
            bursts += 1;
            if std::rc::Rc::ptr_eq(&kept, &cache.1) {
                cached_bursts += 1;
            } else {
                // The guests came or went: bodies built afresh stand for the rest of the tick.
                cache = built_bodies(&guests);
            }
        }
    }
    assert!(
        cached_bursts * 3 > bursts,
        "the cached bodies served most bursts ({cached_bursts} of {bursts})"
    );
}

/// What a tick of the scenario left: the boxes the Builder's turn sees from the cache and from
/// bodies built afresh, and each guest's position, health and mode as bits.
type Seen = (bool, Boxes, Boxes, Vec<(u32, [u64; 3], u32, i32, bool)>);

/// Guests teleporting, hitting, pushing, dying and changing mode, tick after tick, with the
/// cache refreshed by write or (`every_guest`) for every guest after each turn.
fn scenario(every_guest: bool) -> Vec<Seen> {
    super::super::REFRESH_EVERY_GUEST.set(every_guest);
    let mut resources = None;
    let mut game = game(&mut resources);
    let script = [
        "tp Mover 13.5 100 8.5",
        "damage Mover 2 minecraft:player_attack by Builder",
        "damage Builder 3 minecraft:player_attack by Mover",
        "tp Builder 8.5 100 8.5",
        "effect give Mover minecraft:levitation 1 3",
        "gamemode creative Builder",
        "damage Mover 1 minecraft:explosion",
        "gamemode spectator Mover",
        "tp Mover 2.5 100 2.5",
        "gamemode survival Mover",
        "kill Builder",
        "tp Mover 30.5 100 14.5",
        "damage Mover 30 minecraft:generic",
        "tp Builder 20.5 100 4.5",
        "effect give Builder minecraft:slowness 2 2",
        "damage Builder 5 minecraft:player_attack by Mover",
    ];
    let mut log = Vec::new();
    for step in 0..40 {
        let ok = game.run_command(script[step % script.len()]);
        crate::game::lan_actions::building_cache(game.lan.as_ref());
        game.tick_guests_inner();
        let cached = seen_by_builder(&mut game);
        crate::game::lan_actions::building_cache(None);
        let fresh = seen_by_builder(&mut game);
        let guests = game
            .lan
            .as_ref()
            .unwrap()
            .hub
            .guests
            .iter()
            .map(|g| {
                let [x, y, z] = g.position.map(f64::to_bits);
                (g.conn, [x, y, z], g.health.to_bits(), g.game_mode, g.dead)
            })
            .collect();
        log.push((ok, cached, fresh, guests));
    }
    super::super::REFRESH_EVERY_GUEST.set(false);
    log
}

#[test]
#[ignore = "daily"]
fn a_scenario_of_guests_hitting_pushing_and_teleporting_is_the_same_by_write_or_for_every_guest() {
    let by_write = scenario(false);
    let every = scenario(true);
    assert_eq!(by_write.len(), every.len());
    let changes = by_write.iter().filter(|s| s.1 != by_write[0].1).count();
    assert!(changes > 5, "the scenario moved the boxes ({changes} ticks)");
    for (tick, (a, b)) in by_write.iter().zip(&every).enumerate() {
        assert_eq!(a, b, "tick {tick}");
    }
}

/// What the guests' turns of one tick cost in bodies, the way they were kept before (each turn
/// checks the cells against the guest list, guest by guest, and refreshes every cell) and by
/// write: `cargo test --release -p horizon-client --lib -- --ignored --nocapture bodies_cost`.
#[test]
#[ignore = "daily"]
fn bodies_cost() {
    use horizon_client::lan::guest_list::GuestList;
    for n in [50u32, 100, 200, 400, 800] {
        let mut guests = GuestList::default();
        for conn in 1..=n {
            guests.push(listed_guest(conn));
        }
        let cache = built_bodies(&guests);
        let ticks = (2_000_000 / (n * n)).max(3);
        let (mut old_ns, mut new_ns) = (0u128, 0u128);
        for tick in 0..ticks {
            let start = std::time::Instant::now();
            for turn in 0..n as usize {
                // The turn: its guest moves; the cells are checked, then refreshed.
                guests.update(turn, |g| g.position[0] += 0.05 * f64::from(tick % 3));
                let live: Vec<_> = guests.simulation_iter().collect();
                let still = cache.1.len() == live.len()
                    && cache
                        .1
                        .iter()
                        .zip(&live)
                        .all(|(p, g)| p.get().conn == Some(g.conn));
                assert!(still);
                for (cell, g) in cache.1.iter().zip(&live) {
                    if cell.get().conn == Some(g.conn) {
                        cell.set(building_player(g));
                    }
                }
            }
            old_ns += start.elapsed().as_nanos();
            let start = std::time::Instant::now();
            for turn in 0..n as usize {
                guests.update(turn, |g| g.position[0] += 0.05 * f64::from(tick % 3));
                assert!(std::rc::Rc::ptr_eq(&bodies_of(&guests, Some(&cache)), &cache.1));
                refresh_bodies(&guests, &cache);
            }
            new_ns += start.elapsed().as_nanos();
        }
        let pairs = f64::from(ticks) * f64::from(n) * f64::from(n - 1);
        println!(
            "{n:4} guests: {:7.1} ns a pair before, {:5.1} ns by write ({:.0} us a tick before, {:.0} us by write)",
            old_ns as f64 / pairs,
            new_ns as f64 / pairs,
            old_ns as f64 / f64::from(ticks) / 1e3,
            new_ns as f64 / f64::from(ticks) / 1e3,
        );
    }
}
