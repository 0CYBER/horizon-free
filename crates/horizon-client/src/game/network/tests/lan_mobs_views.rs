//! The mobs' view of a guest, made by swapping only the fields it reads in, is the one a turn of
//! the guest gives, tick by tick: every guest's record and body, what each connection was sent,
//! the scoreboard and the views themselves, with the guest's inputs changing (armour, hands,
//! effects, sneak, bed, mode, team, look) and with each condition that sends a view through the
//! turn played, the path counted.
use crate::game::*;
use horizon_client::{
    chunk::Chunk,
    lan::{Joined, NetConfig, Out, PlayerSender, guest::Guest, hub::HubConfig},
    motion::PlayerMotion,
};
use horizon_protocol::packets::configuration::ClientInformation;
use std::fmt::Write as _;
use std::sync::{Arc, atomic::AtomicI32, mpsc, mpsc::Receiver};

/// A game with a guest for each `(connection, name, x)`, standing on a stone floor, and what
/// each one is sent.
fn game_of<'a>(
    resources: &'a mut Option<std::sync::Arc<crate::game::Resources>>,
    guests: &[(u32, &str, f64)],
) -> (Game<'a>, Vec<(u32, mpsc::Receiver<Out>)>) {
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
        ..NetConfig::lan(0, "guest-turn-merge-test".into(), 8)
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
            max_players: 8,
            integrated: false,
            levels: Default::default(),
            level_seeds: Default::default(),
        },
    )
    .unwrap();
    let mut received = Vec::new();
    for &(conn, name, x) in guests {
        let (tx, rx) = mpsc::channel();
        received.push((conn, rx));
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
        // The client has loaded: until then the player is invulnerable and its actions ignored.
        guest.load_timeout = 0;
        game.lan.as_mut().unwrap().hub.guests.push(guest);
    }
    (game, received)
}

const NAMES: [(u32, &str, f64); 6] = [
    (2, "Walker", 4.5),
    (3, "Jumper", 8.5),
    (4, "Swimmer", 19.5),
    (5, "Faller", 12.5),
    (6, "Eater", 16.5),
    (7, "Still", 24.5),
];

/// A packet body as the runs can be compared: a sound's last eight bytes are its seed, drawn from
/// entropy, not from the level's random.
fn unseeded(body: &[u8]) -> &[u8] {
    match body.first() {
        Some(&(118 | 119)) if body.len() > 8 => &body[..body.len() - 8],
        _ => body,
    }
}

fn host_equipment_and_effects(game: &Game<'_>) -> String {
    format!(
        "{:?} {:?} {:?} {:?}",
        game.inventory, game.armor, game.offhand, game.potions.effects
    )
}

/// Everything the turns may have changed, as text with the floats as bits.
fn digest(game: &mut Game<'_>, received: &[(u32, Receiver<Out>)], blocks: &[[i32; 3]]) -> String {
    let before = host_equipment_and_effects(game);
    let views = game.lan_guest_views();
    assert_eq!(host_equipment_and_effects(game), before);
    let mut text = String::new();
    let host = game.lan.as_ref().unwrap();
    for g in host.hub.guests.iter() {
        let bits = |v: [f64; 3]| v.map(f64::to_bits);
        let _ = write!(
            text,
            "[{} pos={:?} prev={:?} ground={} hp={:08x} food={} sat={:08x} xp=({},{:08x},{},{}) mode={} dead={} sprint={} sneak={} glide={} veh={:?} sel={} eff={:?} adv={:?}",
            g.conn,
            bits(g.position),
            bits(g.previous_position),
            g.on_ground,
            g.health.to_bits(),
            g.food,
            g.saturation.to_bits(),
            g.xp_level,
            g.xp_progress.to_bits(),
            g.xp_total,
            g.score,
            g.game_mode,
            g.dead,
            g.sprinting,
            g.sneaking,
            g.fall_flying,
            g.vehicle,
            g.selected,
            g.effects,
            g.advancements
                .criteria
                .iter()
                .map(|(a, c)| (a.as_str(), c.keys().collect::<Vec<_>>()))
                .collect::<Vec<_>>(),
        );
        let items: Vec<_> = g
            .inventory
            .iter()
            .chain(g.armor.iter())
            .map(|s| s.as_ref().map(|s| (s.item.to_string(), s.count)))
            .collect();
        let _ = write!(text, " inv={items:?} offhand={:?}", g.offhand);
        if let Some(m) = g.motion.as_ref() {
            let _ = write!(
                text,
                " body=(pos={:?} vel={:?} fall={:08x} ground={} hp={:08x} food={} pose={:?})",
                bits([m.position.x, m.position.y, m.position.z]),
                bits([m.velocity.x, m.velocity.y, m.velocity.z]),
                (m.fall_distance as f32).to_bits(),
                m.ground.on_ground,
                m.health.health().to_bits(),
                m.health.food.food,
                m.pose,
            );
        }
        text.push_str("]\n");
    }
    // What each connection was sent this tick: the packet bodies in order.
    for (conn, rx) in received {
        let _ = write!(text, "sent {conn}:");
        for out in rx.try_iter() {
            match out {
                Out::Packet(body) => {
                    let _ = write!(text, " {:02x?}", unseeded(&body));
                }
                Out::Packets(bodies) => {
                    for body in bodies {
                        let _ = write!(text, " {:02x?}", unseeded(&body));
                    }
                }
                Out::Disconnect(_) => text.push_str(" disconnect"),
            }
        }
        text.push('\n');
    }
    for b in blocks {
        let _ = writeln!(
            text,
            "block {b:?} {:?} {:?}",
            game.world.block(b[0], b[1], b[2]),
            game.world.block_entities.get(&(b[0], b[1], b[2]))
        );
    }
    let extra = EXTRA.with_borrow_mut(std::mem::take);
    let _ = write!(
        text,
        "scores {:?}\nviews {:?}\nextra {extra:?}",
        game.score.board.scores, views
    );
    text
}

/// The level's random from a fixed seed: a sound's seed and what else the game draws are the
/// same in the two runs.
fn seeded(game: &mut Game<'_>) {
    game.world.random = horizon_core::random::LevelRandom::for_level(
        horizon_core::random::LegacyRandom::new(12345),
    );
}

/// Sets a guest's record through the list, as a packet or a command would.
fn rec(game: &mut Game<'_>, conn: u32, f: impl FnOnce(&mut Guest)) {
    let host = game.lan.as_mut().unwrap();
    let i = host.hub.guests.connection_index(conn).unwrap();
    host.hub.guests.update(i, f);
}

thread_local! {
    /// What views asked inside a guest's turn answered, for the digest.
    static EXTRA: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

use super::{FULL, SKIP, VIEWS};
use horizon_client::motion::MotionEvent;

fn nothing(_: &mut Game<'_>) {}

fn cmd(game: &mut Game<'_>, command: &str) {
    if !game.run_command(command) {
        eprintln!("  (the game answered failure to: {command})");
    }
}

/// A scenario: what happens to the guests when, and what path Walker's view must take.
struct Case {
    name: &'static str,
    setup: fn(&mut Game<'_>),
    /// Before the tick's turns.
    step: fn(&mut Game<'_>, usize),
    /// After the tick's turns, before the views are asked.
    after: fn(&mut Game<'_>, usize),
    /// (tick from, tick to excluded, whether Walker's view is made by the light path).
    expect: &'static [(usize, usize, bool)],
}

const TICKS: usize = 24;

fn motion_mut(game: &mut Game<'_>, conn: u32, f: impl FnOnce(&mut PlayerMotion)) {
    rec(game, conn, |g| {
        if let Some(m) = g.motion.as_mut() {
            f(m)
        }
    });
}

const CASES: &[Case] = &[
    Case {
        name: "quiet",
        setup: nothing,
        step: |_, _| {},
        after: |_, _| {},
        expect: &[(0, TICKS, true)],
    },
    Case {
        name: "the inputs of the view change one after the other",
        setup: |game| cmd(game, "team add reds"),
        step: |game, tick| match tick {
            3 => cmd(
                game,
                "item replace entity Walker armor.chest with minecraft:iron_chestplate",
            ),
            4 => cmd(
                game,
                "item replace entity Walker armor.head with minecraft:carved_pumpkin",
            ),
            5 => cmd(
                game,
                "item replace entity Walker weapon.mainhand with minecraft:diamond_sword",
            ),
            6 => cmd(
                game,
                "item replace entity Walker weapon.offhand with minecraft:shield",
            ),
            7 => cmd(game, "effect give Walker minecraft:invisibility 5 0"),
            8 => cmd(game, "effect give Walker minecraft:glowing 5 0"),
            9 => rec(game, 2, |g| g.sneaking = true),
            10 => rec(game, 2, |g| g.sneaking = false),
            11 => cmd(game, "team join reds Walker"),
            12 => cmd(game, "team modify reds color red"),
            13 => rec(game, 2, |g| g.sprinting = true),
            14 => rec(game, 2, |g| {
                g.yaw = 90.0;
                g.pitch = 30.0;
            }),
            15 => cmd(game, "gamemode creative Walker"),
            16 => cmd(game, "gamemode survival Walker"),
            17 => cmd(
                game,
                "item replace entity Walker armor.head with minecraft:golden_helmet",
            ),
            _ => {}
        },
        after: |_, _| {},
        expect: &[(0, TICKS, true)],
    },
    Case {
        name: "the guest is dead",
        setup: nothing,
        step: |game, tick| {
            if tick == 8 {
                cmd(game, "kill Walker");
            }
        },
        after: |_, _| {},
        expect: &[],
    },
    Case {
        name: "the record's health is not the body's",
        setup: nothing,
        step: |_, _| {},
        after: |game, tick| {
            // The next turn takes the record's health into the body: one view only differs.
            if tick == 8 {
                rec(game, 2, |g| g.health = 5.0);
            }
        },
        expect: &[(0, 8, true), (8, 9, false), (9, TICKS, true)],
    },
    Case {
        name: "the body is hurt from a direction",
        setup: nothing,
        step: |_, _| {},
        after: |game, tick| {
            if (8..12).contains(&tick) {
                motion_mut(game, 2, |m| m.health.indicated_dir = Some(45.0));
            }
        },
        expect: &[(0, 8, true), (8, 12, false)],
    },
    Case {
        name: "the body was teleported",
        setup: nothing,
        step: |_, _| {},
        after: |game, tick| {
            if (8..12).contains(&tick) {
                motion_mut(game, 2, |m| m.teleported = true);
            }
        },
        expect: &[(0, 8, true), (8, 12, false)],
    },
    Case {
        name: "the body has events nobody took",
        setup: nothing,
        step: |_, _| {},
        after: |game, tick| {
            if (8..12).contains(&tick) {
                motion_mut(game, 2, |m| m.events.push(MotionEvent::InsideWitherRose));
            }
        },
        expect: &[(0, 8, true), (8, 12, false)],
    },
    Case {
        name: "the guest has no body yet",
        setup: nothing,
        step: |_, _| {},
        after: |game, tick| {
            if tick == 8 {
                rec(game, 2, |g| {
                    g.new_body(true);
                });
            }
        },
        expect: &[(0, 8, true), (8, 9, false)],
    },
    Case {
        name: "the guest has no damage record",
        setup: nothing,
        step: |_, _| {},
        after: |game, tick| {
            if tick == 8 {
                game.guest_combat.remove(&2);
            }
        },
        expect: &[(0, 8, true), (8, 9, false)],
    },
    Case {
        name: "the guest is a fake one",
        setup: nothing,
        step: |_, _| {},
        after: |game, tick| {
            if tick == 8 {
                rec(game, 2, |g| {
                    g.kind = horizon_client::lan::guest::GuestKind::Fake
                });
            }
        },
        expect: &[(0, 8, true)],
    },
    Case {
        name: "the views are asked inside another guest's turn",
        setup: nothing,
        step: |_, _| {},
        after: |game, _| {
            VIEWS.with_borrow_mut(Vec::clear);
            let nested = game
                .as_guest(3, |g| format!("{:?}", g.lan_guest_views()))
                .unwrap();
            EXTRA.with_borrow_mut(|e| e.push(nested));
            // Inside a guest's turn the view of every guest goes through a turn.
            assert!(VIEWS.with_borrow(|v| !v.is_empty() && v.iter().all(|p| !p.1)));
        },
        expect: &[],
    },
];

struct Run {
    log: Vec<String>,
    /// For each tick, the (connection, light) of the views made.
    views: Vec<Vec<(u32, bool)>>,
}

fn play(case: &Case, light: bool, skip: u64) -> Run {
    FULL.set(!light);
    SKIP.set(skip);
    let mut resources = None;
    let (mut game, received) = game_of(&mut resources, &NAMES[..3]);
    seeded(&mut game);
    (case.setup)(&mut game);
    let mut run = Run {
        log: Vec::new(),
        views: Vec::new(),
    };
    for tick in 0..TICKS {
        (case.step)(&mut game, tick);
        game.tick_guests();
        (case.after)(&mut game, tick);
        VIEWS.with_borrow_mut(Vec::clear);
        run.log.push(digest(&mut game, &received, &[]));
        run.views.push(VIEWS.with_borrow(|v| v.clone()));
    }
    FULL.set(false);
    SKIP.set(0);
    run
}

fn difference(old: &[String], new: &[String]) -> Option<String> {
    old.iter().zip(new).enumerate().find_map(|(tick, (a, b))| {
        (a != b).then(|| {
            let at = a
                .bytes()
                .zip(b.bytes())
                .position(|(x, y)| x != y)
                .unwrap_or(a.len().min(b.len()));
            let from = at.saturating_sub(120);
            format!(
                "tick {tick} byte {at}:\n old ...{}\n new ...{}",
                &a[from..(at + 160).min(a.len())],
                &b[from..(at + 160).min(b.len())]
            )
        })
    })
}

fn path_of(run: &Run, tick: usize) -> Option<bool> {
    run.views[tick].iter().find(|v| v.0 == 2).map(|v| v.1)
}

#[test]
#[ignore = "daily"]
fn a_panicking_light_view_restores_the_host_and_guest() {
    use std::panic::{AssertUnwindSafe, catch_unwind};

    let mut resources = None;
    let (mut game, _received) = game_of(&mut resources, &NAMES[..1]);
    game.tick_guests();
    game.offhand = Some(ItemStack::new("minecraft:torch", 1));
    rec(&mut game, 2, |guest| {
        guest.offhand = Some(ItemStack::new("minecraft:shield", 1));
    });
    let before = host_equipment_and_effects(&game);
    let host_position = game.player.as_ref().unwrap().position;

    VIEWS.with_borrow_mut(Vec::clear);
    let result = catch_unwind(AssertUnwindSafe(|| {
        game.guest_player_view(2, |_| std::panic::panic_any("view panic"))
    }));
    assert_eq!(
        result.unwrap_err().downcast_ref::<&str>(),
        Some(&"view panic")
    );
    VIEWS.with_borrow(|views| assert_eq!(views.as_slice(), &[(2, true)]));
    assert_eq!(host_equipment_and_effects(&game), before);
    assert_eq!(game.player.as_ref().unwrap().position, host_position);
    assert_eq!(game.lan_acting, None);
    assert!(game.guest_combat.contains_key(&2));
    let guest = game.lan.as_ref().unwrap().hub.guest(2).unwrap();
    assert!(guest.motion.is_some());
    assert_eq!(guest.offhand, Some(ItemStack::new("minecraft:shield", 1)));
}

#[test]
#[ignore = "daily"]
fn the_light_view_is_the_view_of_a_turn_and_each_fallback_is_taken_where_it_should() {
    for case in CASES {
        let old = play(case, false, 0);
        assert!(
            old.views.iter().flatten().all(|v| !v.1),
            "the old path only"
        );
        let new = play(case, true, 0);
        if let Some(d) = difference(&old.log, &new.log) {
            panic!("{}: {d}", case.name);
        }
        let shown: String = (0..TICKS)
            .map(|t| match path_of(&new, t) {
                Some(true) => 'L',
                Some(false) => 't',
                None => '-',
            })
            .collect();
        eprintln!("{:60} Walker's view: {shown}", case.name);
        for &(from, to, light) in case.expect {
            for tick in from..to {
                assert_eq!(
                    path_of(&new, tick),
                    Some(light),
                    "{}: tick {tick} should be {}, views {shown}",
                    case.name,
                    if light { "light" } else { "through a turn" }
                );
            }
        }
    }
}

/// The digest sees what the view is made of: each input of the view changes the views at the
/// tick it changes (so equality of the two paths says something about it).
#[test]
#[ignore = "daily"]
fn the_views_change_when_their_inputs_do() {
    let run = play(&CASES[1], false, 0);
    let views: Vec<&str> = run
        .log
        .iter()
        .map(|l| l.rsplit("\nviews ").next().unwrap_or(""))
        .collect();
    let changed: Vec<usize> = (1..TICKS).filter(|&t| views[t] != views[t - 1]).collect();
    eprintln!("the views change at ticks {changed:?}");
    for tick in [3, 4, 5, 6, 7, 8, 9, 10, 13, 14, 15, 16, 17] {
        assert!(
            changed.contains(&tick),
            "no change at tick {tick}: {changed:?}"
        );
    }
}

fn conditions() -> Vec<(u32, &'static str)> {
    vec![
        (0, "the guest has a body"),
        (1, "not dead"),
        (2, "health above 0"),
        (3, "the body's health is the record's"),
        (4, "the body is not dying"),
        (5, "no hurt direction"),
        (6, "not teleported"),
        (7, "no events"),
        (8, "a network guest"),
        (10, "a damage record"),
        (11, "no guest is acting"),
        (12, "no swap in progress"),
    ]
}

/// Which scenario each condition is needed by (a report; `CONDS=3,4` runs some only). Taken away
/// one at a time, then all together.
#[test]
#[ignore = "daily"]
fn report_which_scenario_each_condition_is_needed_by() {
    // A report of about 3 minutes a condition: run with CONDS=3,4, FORCED=1 or ALL=1.
    if !["CONDS", "FORCED", "ALL"]
        .iter()
        .any(|v| std::env::var_os(v).is_some())
    {
        return;
    }
    let shard: Vec<u32> = std::env::var("CONDS")
        .map(|v| v.split(',').filter_map(|n| n.parse().ok()).collect())
        .unwrap_or_default();
    let olds: Vec<Run> = CASES.iter().map(|c| play(c, false, 0)).collect();
    let mut all = 0u64;
    for (i, _) in conditions() {
        all |= 1 << i;
    }
    if std::env::var_os("FORCED").is_some() {
        for (case, old) in CASES.iter().zip(&olds) {
            let new = play(case, true, all);
            let shown = difference(&old.log, &new.log).unwrap_or_else(|| "same".to_owned());
            eprintln!("FORCED {:60} {shown}", case.name);
        }
        return;
    }
    let broke = |skip: u64| -> Vec<&'static str> {
        CASES
            .iter()
            .zip(&olds)
            .filter(|(case, old)| {
                let result = std::panic::catch_unwind(|| {
                    let new = play(case, true, skip);
                    difference(&old.log, &new.log).is_some()
                });
                FULL.set(false);
                SKIP.set(0);
                result.unwrap_or(true)
            })
            .map(|(case, _)| case.name)
            .collect()
    };
    if shard.is_empty() {
        eprintln!("NO CONDITION AT ALL breaks {:?}", broke(all));
    }
    for (i, name) in conditions() {
        if !shard.is_empty() && !shard.contains(&i) {
            continue;
        }
        eprintln!("{i:2} {name:40} breaks {:?}", broke(1 << i));
    }
}
