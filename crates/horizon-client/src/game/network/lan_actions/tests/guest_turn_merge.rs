//! The guests' turns of a whole scenario are the same state, tick by tick, whether the sculk step and
//! the tail of the guest's tick run in the turn of its body (when it told no one anything) or each
//! in a turn of its own: every guest's record and body, what each connection was sent, the
//! scoreboard, the mobs' views of the guests. Players walk, jump, swim, fall, are hurt, healed,
//! given items, effects and an advancement, teleported and put in other modes.
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

/// Everything the turns may have changed, as text with the floats as bits.
fn digest(game: &mut Game<'_>, received: &[(u32, Receiver<Out>)], blocks: &[[i32; 3]]) -> String {
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
        let _ = write!(text, " inv={items:?}");
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
    // The mobs' views of the guests, as their turns give them.
    for b in blocks {
        let _ = writeln!(
            text,
            "block {b:?} {:?} {:?}",
            game.world.block(b[0], b[1], b[2]),
            game.world.block_entities.get(&(b[0], b[1], b[2]))
        );
    }
    let views = game.lan_guest_views();
    let _ = write!(
        text,
        "scores {:?}\nviews {:?}",
        game.score.board.scores, views
    );
    text
}

/// Moves a guest by `delta` as a client's tick would send it.
fn walk(game: &mut Game<'_>, conn: u32, delta: [f64; 3], on_ground: bool) {
    let host = game.lan.as_mut().unwrap();
    let i = host.hub.guests.connection_index(conn).unwrap();
    host.hub.guests.update(i, |g| {
        g.previous_position = g.position;
        for k in 0..3 {
            g.position[k] += delta[k];
        }
        g.moves.push((delta, on_ground));
        g.moves_from_ground = g.on_ground;
        g.on_ground = on_ground;
    });
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

/// Which path a guest's body turn took, as the tests expect it.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Path {
    /// The sculk step and the tail ran in the body's turn.
    Merged,
    /// They did not (a turn of their own, or no body turn at all).
    Apart,
}

/// A scenario of the fallbacks: what happens to guest 2 (Walker) when, and what the gate must do.
struct Case {
    name: &'static str,
    /// Blocks whose states the digest follows.
    blocks: &'static [[i32; 3]],
    setup: fn(&mut Game<'_>),
    step: fn(&mut Game<'_>, usize),
    /// (tick from, tick to excluded, path Walker's body turn must take in them).
    expect: &'static [(usize, usize, Path)],
}

const TICKS: usize = 30;
/// The player is left with 3 health and stands in lava from tick 3: the hazard kills it in the
/// body's own turn.
fn fall_step(game: &mut Game<'_>, tick: usize) {
    if tick == 1 {
        cmd(game, "damage Walker 17 minecraft:generic");
    }
    if tick == 3 {
        cmd(game, "setblock 4 100 8 minecraft:lava");
    }
}

/// What a guest's client sends, through the hub's own inbox and handlers (not fields set by
/// hand): the packets, then the end of the client's tick.
fn send(
    game: &mut Game<'_>,
    conn: u32,
    packets: Vec<horizon_protocol::packets::play::Serverbound>,
) {
    use horizon_client::lan::guest::InboxItem;
    rec(game, conn, |g| {
        for p in packets {
            g.inbox.push_back(InboxItem::Packet(p, None));
        }
    });
    game.tick_lan_packets();
}

fn command_packet(game: &mut Game<'_>, conn: u32, action: i32) {
    use horizon_protocol::packets::play::{self, Serverbound as S};
    let id = game.command_guest(conn).unwrap().entity_id;
    send(
        game,
        conn,
        vec![S::PlayerCommand(play::PlayerCommand {
            id,
            action,
            data: 0,
        })],
    );
}

/// A client tick that moved by `delta`, as its packets say.
fn move_packet(game: &mut Game<'_>, conn: u32, delta: [f64; 3], ground: bool) {
    use horizon_protocol::packets::{Empty, play, play::Serverbound as S};
    let at = game.command_guest(conn).unwrap().position;
    send(
        game,
        conn,
        vec![
            S::MovePlayerPos(play::MovePlayerPos {
                x: at[0] + delta[0],
                y: at[1] + delta[1],
                z: at[2] + delta[2],
                flags: u8::from(ground),
            }),
            S::ClientTickEnd(Empty),
        ],
    );
}

/// Puts a guest in the air at `at`, where its client would be after a move there.
fn place(game: &mut Game<'_>, conn: u32, at: [f64; 3]) {
    rec(game, conn, |g| {
        g.position = at;
        g.previous_position = at;
        g.on_ground = false;
    });
}

/// The objectives every scenario reads and writes.
fn objectives(game: &mut Game<'_>) {
    for (name, criterion) in [
        ("hp", "health"),
        ("fd", "food"),
        ("ar", "armor"),
        ("ai", "air"),
        ("lv", "level"),
        ("xp", "xp"),
        ("wk", "minecraft.custom:minecraft.walk_one_cm"),
        ("sp", "minecraft.custom:minecraft.sprint_one_cm"),
        ("jm", "minecraft.custom:minecraft.jump"),
        ("fl", "minecraft.custom:minecraft.fall_one_cm"),
        ("dt", "minecraft.custom:minecraft.damage_taken"),
        ("pt", "minecraft.custom:minecraft.play_time"),
    ] {
        cmd(
            game,
            &format!("scoreboard objectives add {name} {criterion}"),
        );
    }
}

fn nothing(_: &mut Game<'_>) {}

/// Runs a command; one the game answers with failure (a `damage` a totem undid, a `ride` with
/// nothing to ride) is told, not fatal: what the scenario then shows is what is compared.
fn cmd(game: &mut Game<'_>, command: &str) {
    if !game.run_command(command) {
        eprintln!("  (the game answered failure to: {command})");
    }
}

const CASES: &[Case] = &[
    Case {
        name: "quiet",
        blocks: &[],
        setup: nothing,
        step: |_, _| {},
        expect: &[(4, TICKS, Path::Merged)],
    },
    Case {
        name: "walking and sprinting",
        blocks: &[],
        setup: nothing,
        step: |game, tick| {
            if tick >= 5 {
                rec(game, 2, |g| g.sprinting = true);
                walk(game, 2, [0.2, 0.0, 0.1], true);
            }
        },
        expect: &[(8, TICKS, Path::Merged)],
    },
    Case {
        name: "death",
        blocks: &[],
        setup: nothing,
        step: |game, tick| {
            if tick == 8 {
                cmd(game, "kill Walker");
            }
        },
        expect: &[(4, 8, Path::Merged), (8, 9, Path::Apart)],
    },
    Case {
        name: "totem",
        blocks: &[],
        setup: nothing,
        step: |game, tick| match tick {
            5 => cmd(
                game,
                "item replace entity Walker weapon.offhand with minecraft:totem_of_undying",
            ),
            8 => cmd(game, "damage Walker 100 minecraft:generic"),
            _ => {}
        },
        expect: &[(4, 8, Path::Merged), (8, 9, Path::Apart)],
    },
    Case {
        name: "hit by a mob",
        blocks: &[],
        setup: nothing,
        step: |game, tick| match tick {
            5 => {
                game.options.dedicated = Some(Vec::new());
                let id = game.summon(
                    game.dim.id(),
                    "minecraft:zombie",
                    [6.5, 100.0, 8.5],
                    Default::default(),
                );
                eprintln!("  summon zombie: {id:?}");
            }
            8 => cmd(
                game,
                "damage Walker 3 minecraft:mob_attack by @e[type=minecraft:zombie,limit=1]",
            ),
            _ => {}
        },
        expect: &[(8, 9, Path::Apart)],
    },
    Case {
        name: "riding a boat",
        blocks: &[],
        setup: nothing,
        step: |game, tick| match tick {
            5 => {
                game.options.dedicated = Some(Vec::new());
                let id = game.summon(
                    game.dim.id(),
                    "minecraft:boat",
                    [4.5, 100.0, 8.5],
                    Default::default(),
                );
                eprintln!("  summon boat: {id:?}");
                rec(game, 2, |g| g.vehicle = id);
            }
            16 => rec(game, 2, |g| g.vehicle = None),
            _ => {}
        },
        expect: &[],
    },
    Case {
        name: "sleeping",
        blocks: &[[4, 100, 9]],
        setup: nothing,
        step: |game, tick| {
            if tick == 5 {
                cmd(
                    game,
                    "setblock 4 100 9 minecraft:red_bed[part=foot,facing=south]",
                );
                cmd(
                    game,
                    "setblock 4 100 10 minecraft:red_bed[part=head,facing=south]",
                );
                rec(game, 2, |g| {
                    g.bed = Some([4, 100, 9]);
                    g.sleeping = true;
                });
            }
            if tick == 20 {
                rec(game, 2, |g| {
                    g.bed = None;
                    g.sleeping = false;
                });
            }
        },
        expect: &[],
    },
    Case {
        name: "gliding in elytra",
        blocks: &[],
        setup: nothing,
        step: |game, tick| {
            if tick == 3 {
                cmd(
                    game,
                    "item replace entity Walker armor.chest with minecraft:elytra",
                );
                rec(game, 2, |g| {
                    g.position[1] = 112.0;
                    g.on_ground = false;
                    g.fall_flying = true;
                });
            }
            if tick >= 4 && game.lan.as_ref().unwrap().hub.guests[0].position[1] > 100.5 {
                walk(game, 2, [0.0, -0.3, 0.2], false);
            }
        },
        expect: &[],
    },
    Case {
        name: "spectator",
        blocks: &[],
        setup: nothing,
        step: |game, tick| {
            if tick == 5 {
                cmd(game, "gamemode spectator Walker");
            }
        },
        expect: &[(4, 5, Path::Merged), (6, TICKS, Path::Apart)],
    },
    Case {
        name: "sculk sensor under the player",
        blocks: &[[4, 99, 8]],
        setup: |game| cmd(game, "setblock 4 99 8 minecraft:sculk_sensor"),
        step: |game, tick| {
            if tick % 6 == 2 {
                walk(game, 2, [0.0, 0.0, 0.0], true);
            }
        },
        expect: &[],
    },
    Case {
        name: "an effect",
        blocks: &[],
        setup: nothing,
        step: |game, tick| {
            if tick == 5 {
                cmd(game, "effect give Walker minecraft:speed 3 0");
            }
        },
        expect: &[(4, 5, Path::Merged), (6, 12, Path::Apart)],
    },
    Case {
        name: "a teleport and a mode",
        blocks: &[],
        setup: nothing,
        step: |game, tick| match tick {
            8 => cmd(game, "tp Walker 6.5 100 6.5"),
            12 => cmd(game, "gamemode creative Walker"),
            16 => cmd(game, "gamemode survival Walker"),
            _ => {}
        },
        expect: &[(8, 9, Path::Apart), (12, 13, Path::Apart)],
    },
    Case {
        name: "real sprint packets",
        blocks: &[],
        setup: nothing,
        step: |game, tick| {
            if tick == 5 {
                command_packet(game, 2, 1);
            }
            if tick >= 5 {
                move_packet(game, 2, [0.2, 0.0, 0.1], true);
            }
            if tick == 15 {
                command_packet(game, 2, 2);
            }
        },
        expect: &[],
    },
    Case {
        name: "real glide packets",
        blocks: &[],
        setup: nothing,
        step: |game, tick| {
            match tick {
                2 => cmd(
                    game,
                    "item replace entity Walker armor.chest with minecraft:elytra",
                ),
                3 => place(game, 2, [4.5, 112.0, 8.5]),
                6 => command_packet(game, 2, 6),
                _ => {}
            }
            if (7..20).contains(&tick) {
                move_packet(game, 2, [0.0, -0.4, 0.3], false);
            }
        },
        expect: &[],
    },
    Case {
        name: "real sleep packets",
        blocks: &[[4, 100, 9]],
        setup: |game| {
            cmd(
                game,
                "setblock 4 100 9 minecraft:red_bed[part=foot,facing=south]",
            );
            cmd(
                game,
                "setblock 4 100 10 minecraft:red_bed[part=head,facing=south]",
            );
            cmd(game, "time set night");
        },
        step: |game, tick| {
            use horizon_protocol::packets::play::{self, Serverbound as S};
            if tick == 5 {
                send(
                    game,
                    2,
                    vec![S::UseItemOn(play::UseItemOn {
                        hand: 0,
                        pos: [4, 100, 9],
                        face: 1,
                        cursor: [0.5, 0.5, 0.5],
                        inside: false,
                        world_border: false,
                        sequence: 1,
                    })],
                );
            }
            if tick == 20 {
                command_packet(game, 2, 0);
            }
        },
        expect: &[],
    },
    Case {
        name: "real mount packets",
        blocks: &[],
        setup: nothing,
        step: |game, tick| {
            use horizon_protocol::packets::play::{self, Serverbound as S};
            if tick == 4 {
                game.options.dedicated = Some(Vec::new());
                let id = game.summon(
                    game.dim.id(),
                    "minecraft:oak_boat",
                    [4.5, 100.0, 8.5],
                    Default::default(),
                );
                eprintln!("  summon oak_boat: {id:?}");
                if let Some(id) = id {
                    send(
                        game,
                        2,
                        vec![S::Interact(play::Interact {
                            id: id as i32,
                            hand: 0,
                            location: [0.0; 3],
                            secondary: false,
                        })],
                    );
                }
            }
        },
        expect: &[],
    },
    Case {
        name: "hurt and attributes changed in the same tick",
        blocks: &[],
        setup: nothing,
        step: |game, tick| {
            if tick == 8 {
                cmd(
                    game,
                    "attribute Walker minecraft:movement_speed base set 0.2",
                );
                cmd(game, "damage Walker 3 minecraft:generic");
            }
        },
        expect: &[],
    },
    Case {
        name: "hurt by the other player, who walks",
        blocks: &[],
        setup: nothing,
        step: |game, tick| {
            if tick >= 5 {
                walk(game, 3, [0.1, 0.0, 0.0], true);
            }
            if tick == 8 {
                cmd(game, "damage Walker 3 minecraft:player_attack by Jumper");
                cmd(
                    game,
                    "attribute Jumper minecraft:movement_speed base set 0.2",
                );
            }
        },
        expect: &[],
    },
    Case {
        name: "xp and health read by the other",
        blocks: &[],
        setup: nothing,
        step: |game, tick| {
            if tick == 6 {
                cmd(game, "xp add Walker 30 points");
                cmd(game, "scoreboard players operation Jumper hp = Walker hp");
            }
            if tick == 9 {
                cmd(game, "effect give Walker minecraft:instant_damage 1 0");
            }
        },
        expect: &[],
    },
    Case {
        name: "lava kills a player left with 3 health",
        blocks: &[[4, 100, 8]],
        setup: nothing,
        step: fall_step,
        expect: &[],
    },
    Case {
        name: "lava and a totem, 3 health left",
        blocks: &[[4, 100, 8]],
        setup: |game| {
            cmd(
                game,
                "item replace entity Walker weapon.offhand with minecraft:totem_of_undying",
            )
        },
        step: fall_step,
        expect: &[],
    },
    Case {
        name: "standing in lava",
        blocks: &[[4, 100, 8]],
        setup: nothing,
        step: |game, tick| {
            if tick == 3 {
                cmd(game, "setblock 4 100 8 minecraft:lava");
            }
        },
        expect: &[],
    },
    Case {
        name: "hunger",
        blocks: &[],
        setup: nothing,
        step: |game, tick| {
            if tick == 4 {
                rec(game, 2, |g| {
                    g.food = 9;
                    g.saturation = 0.0;
                });
                cmd(game, "effect give Walker minecraft:hunger 1 200");
            }
        },
        expect: &[],
    },
];

struct Run {
    log: Vec<String>,
    /// For each tick, the (connection, merged) of the body turns.
    trace: Vec<Vec<(u32, bool)>>,
    /// Walker's health at the end of each tick.
    hp: Vec<(f32, f64)>,
    /// Whether Walker rides, sleeps, glides, sprints at the end of each tick.
    states: Vec<[bool; 4]>,
}

fn play(case: &Case, merge: bool, skip: u64) -> Run {
    super::super::guest_body::NO_MERGE.set(!merge);
    super::super::guest_body::SKIP.set(skip);
    let mut resources = None;
    let (mut game, received) = game_of(&mut resources, &NAMES[..3]);
    seeded(&mut game);
    objectives(&mut game);
    (case.setup)(&mut game);
    let mut run = Run {
        log: Vec::new(),
        trace: Vec::new(),
        hp: Vec::new(),
        states: Vec::new(),
    };
    for tick in 0..TICKS {
        (case.step)(&mut game, tick);
        super::super::guest_body::TRACE.with_borrow_mut(Vec::clear);
        game.tick_guests();
        run.trace
            .push(super::super::guest_body::TRACE.with_borrow(|t| t.clone()));
        let w = &game.lan.as_ref().unwrap().hub.guests[0];
        run.hp.push((w.health, w.position[1]));
        run.states.push([
            w.vehicle.is_some(),
            w.bed.is_some(),
            w.fall_flying,
            w.sprinting,
        ]);
        run.log.push(digest(&mut game, &received, case.blocks));
    }
    super::super::guest_body::NO_MERGE.set(false);
    super::super::guest_body::SKIP.set(0);
    run
}

/// The first tick at which the two logs differ, with what differs.
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

#[test]
#[ignore = "daily"]
fn every_case_the_gate_sends_to_the_old_path_runs_the_same_and_takes_the_path_expected() {
    let only = std::env::var("CASE").unwrap_or_default();
    for case in CASES.iter().filter(|c| c.name.contains(&only)) {
        let old = play(case, false, 0);
        assert!(
            old.trace.iter().flatten().all(|t| !t.1),
            "{}: the old run merges nothing",
            case.name
        );
        let new = play(case, true, 0);
        if let Some(d) = difference(&old.log, &new.log) {
            panic!("{}: {d}", case.name);
        }
        let shown: Vec<String> = new
            .trace
            .iter()
            .map(|t| match t.iter().find(|e| e.0 == 2) {
                Some(&(_, true)) => 'M',
                Some(&(_, false)) => 'a',
                None => '-',
            })
            .map(String::from)
            .collect();
        eprintln!("{:32} Walker's body turns: {}", case.name, shown.concat());
        for &(from, to, path) in case.expect {
            for tick in from..to {
                let merged = new.trace[tick].contains(&(2, true));
                assert_eq!(
                    merged,
                    path == Path::Merged,
                    "{}: tick {tick} should be {path:?}, trace {shown:?}",
                    case.name
                );
            }
        }
        // Every case also has the other two guests, still, whose turns merge.
        assert!(
            new.trace.iter().flatten().any(|t| t.0 != 2 && t.1),
            "{}: the still guests merged",
            case.name
        );
    }
}

fn long_run(merge: bool, skip: u64) -> (Vec<String>, usize, usize) {
    super::super::guest_body::NO_MERGE.set(!merge);
    super::super::guest_body::SKIP.set(skip);
    let mut resources = None;
    let (mut game, received) = game_of(&mut resources, &NAMES);
    seeded(&mut game);
    assert!(game.run_command("fill 19 98 8 25 100 8 water"));
    let host = game.lan.as_mut().unwrap();
    host.hub.guests.update(3, |g| {
        g.position[1] = 112.0;
        g.previous_position[1] = 112.0;
        g.on_ground = false;
    });
    // Objectives that follow the statistics, the health and the food the turns change.
    for objective in [
        "walked minecraft.custom:minecraft.walk_one_cm",
        "played minecraft.custom:minecraft.play_time",
        "hp health",
        "meals food",
    ] {
        assert!(
            game.run_command(&format!("scoreboard objectives add {objective}")),
            "{objective}"
        );
    }
    let mut jump = (0.0_f64, 0.0_f64);
    let script: [(usize, &str); 14] = [
        (3, "give Eater minecraft:bread 3"),
        (4, "give Eater minecraft:diamond_chestplate 1"),
        (6, "advancement grant Eater only minecraft:story/root"),
        (8, "effect give Swimmer minecraft:slowness 5 1"),
        (10, "damage Eater 4 minecraft:generic"),
        (12, "gamemode creative Still"),
        (15, "tp Still 3.5 100 3.5"),
        (18, "xp add Eater 30 levels"),
        (20, "effect give Jumper minecraft:jump_boost 3 1"),
        (24, "gamemode survival Still"),
        (28, "damage Walker 3 minecraft:fall"),
        (31, "tp Jumper 20.5 100 12.5"),
        (36, "effect give Eater minecraft:regeneration 4 1"),
        (40, "effect clear Swimmer"),
    ];
    let mut log = Vec::new();
    let (mut apart, mut together) = (0, 0);
    let mut swimmer_touched_water = false;
    super::super::guest_body::TRACE.with_borrow_mut(Vec::clear);
    for tick in 0..60 {
        for &(at, command) in &script {
            if at == tick {
                game.run_command(command);
            }
        }
        walk(&mut game, 2, [0.2, 0.0, 0.1], true);
        // A jump: the vanilla arc, each 20 ticks.
        if tick % 20 == 0 {
            jump = (0.0, 0.42);
        }
        let dy = if jump.1 != 0.0 || jump.0 > 0.0 {
            let before = jump.0;
            jump.0 += jump.1;
            jump.1 = (jump.1 - 0.08) * 0.98;
            if jump.0 <= 0.0 {
                jump = (0.0, 0.0);
            }
            jump.0 - before
        } else {
            0.0
        };
        walk(&mut game, 3, [0.0, dy, 0.0], jump.0 == 0.0);
        walk(&mut game, 4, [0.1, -0.02, 0.0], false);
        if game.lan.as_ref().unwrap().hub.guests[3].position[1] > 100.0 {
            walk(&mut game, 5, [0.0, -0.4, 0.0], false);
        }
        game.tick_guests();
        swimmer_touched_water |= game
            .lan
            .as_ref()
            .unwrap()
            .hub
            .guest(4)
            .and_then(|guest| guest.motion.as_ref())
            .is_some_and(|motion| motion.fluids.was_touching_water);
        log.push(digest(&mut game, &received, &[]));
    }
    super::super::guest_body::TRACE.with_borrow(|t| {
        apart = t.iter().filter(|e| !e.1).count();
        together = t.iter().filter(|e| e.1).count();
    });
    super::super::guest_body::NO_MERGE.set(false);
    super::super::guest_body::SKIP.set(0);
    assert!(
        swimmer_touched_water,
        "Swimmer touched water during the run"
    );
    (log, apart, together)
}

#[test]
#[ignore = "daily"]
fn the_sculk_step_and_the_tail_in_the_bodys_turn_change_nothing() {
    let (old, _, merged_in_old) = long_run(false, 0);
    assert_eq!(merged_in_old, 0, "the old run merges nothing");
    let (new, apart, together) = long_run(true, 0);
    assert!(
        together > 20 && apart > 20,
        "merged {together}, separate {apart}"
    );
    if let Some(d) = difference(&old, &new) {
        panic!("{d}");
    }
    // The scenario really did things: damage, a fall, packets and an advancement.
    let all = old.join("\n");
    assert!(all.contains("story/root"), "the advancement was granted");
    assert!(
        all.contains("walked") && all.contains("played"),
        "the scores follow the statistics"
    );
    assert!(all.contains("sent 2: ["), "the walker was sent packets");
}

/// The conditions of the gate, by their place in `SKIP`.
fn conditions() -> Vec<(u32, String)> {
    let mut all: Vec<(u32, String)> = super::super::guest_body::HOLDS
        .iter()
        .enumerate()
        .map(|(i, c)| (i as u32, (*c).to_owned()))
        .collect();
    let outer = super::super::guest_body::OUTER;
    for (k, name) in [
        "events.iter().all(inert)",
        "velocity.is_none()",
        "soul.is_none()",
    ]
    .into_iter()
    .enumerate()
    {
        all.push((outer + k as u32, name.to_owned()));
    }
    all
}

/// Takes each condition of the gate away, one at a time, and says which scenarios then differ
/// from the old path (a report, no assertion: the conditions that guard the same thing back each
/// other up, so a single one taken away shows only when nothing else catches the case; see
/// `the_gate_is_needed_where_it_matters`). Slow: `CONDS=16,40` runs some only.
#[test]
#[ignore = "daily"]
fn report_which_scenario_each_condition_of_the_gate_is_needed_by() {
    // A report of about 13 minutes a condition: run with CONDS=16,40 or ALL=1.
    if std::env::var_os("CONDS").is_none() && std::env::var_os("ALL").is_none() {
        return;
    }
    let olds: Vec<Run> = CASES.iter().map(|c| play(c, false, 0)).collect();
    let (long_old, _, _) = long_run(false, 0);
    let (rewrite_old, _, _) = listened(false, 0, true);
    // The control: every condition taken away at once. If nothing differs, the scenarios cannot
    // tell a merge that is wrong from one that is right.
    let mut all = 0u64;
    for (i, _) in conditions() {
        all |= 1 << i;
    }
    let mut control: Vec<&str> = Vec::new();
    for (case, old) in CASES.iter().zip(&olds) {
        if difference(&old.log, &play(case, true, all).log).is_some() {
            control.push(case.name);
        }
    }
    eprintln!("CONTROL, no condition at all, differ: {control:?}");
    let mut idle = Vec::new();
    let shard: Vec<u32> = std::env::var("CONDS")
        .map(|v| v.split(',').filter_map(|n| n.parse().ok()).collect())
        .unwrap_or_default();
    for (i, name) in conditions() {
        if !shard.is_empty() && !shard.contains(&i) {
            continue;
        }
        let mut broke: Vec<&str> = Vec::new();
        for (case, old) in CASES.iter().zip(&olds) {
            let new = play(case, true, 1 << i);
            if difference(&old.log, &new.log).is_some() {
                broke.push(case.name);
            }
        }
        let (long_new, _, _) = long_run(true, 1 << i);
        if difference(&long_old, &long_new).is_some() {
            broke.push("long scenario");
        }
        let (rewrite_new, _, _) = listened(true, 1 << i, true);
        if difference(&rewrite_old, &rewrite_new).is_some() {
            broke.push("a mod that keeps the food");
        }
        eprintln!("{i:2} {name:70} breaks {broke:?}");
        if broke.is_empty() {
            idle.push(name);
        }
    }
    eprintln!("NEEDED BY NO SCENARIO: {idle:#?}");
}

/// A mod listening to the hooks a guest's turns fire around the body (`player_tick_pre` and
/// `player_tick_post`, each in a turn of its own) and the one `leave_guest` fires for the food
/// the body's turn changed.
struct Listener(std::sync::Arc<std::sync::Mutex<Vec<String>>>, bool);

#[mod_api::__subscriptions]
impl mod_api::Hooks for Listener {
    fn player_tick_pre(&self, _: mod_api::HostSide<'_>, _: &mut mod_api::PlayerTickPreEvent) {
        self.0.lock().unwrap().push("pre".into());
    }
    fn player_tick_post(&self, _: mod_api::HostSide<'_>, _: &mut mod_api::PlayerTickPostEvent) {
        self.0.lock().unwrap().push("post".into());
    }
    fn food_level_changed(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::FoodLevelChangedEvent<'_>,
    ) -> mod_api::Flow {
        self.0
            .lock()
            .unwrap()
            .push(format!("food {} {}", event.from, event.to));
        if self.1 {
            // A mod that keeps the food where it was: the turn's hunger never reaches the player.
            event.to = event.from;
        }
        mod_api::Flow::Continue
    }
}

/// Two guests of the mods' fixture for 30 ticks, 13 losing food on some of them, with the
/// listener installed: what the guests became, and what the hooks heard, in order.
fn listened(merge: bool, skip: u64, rewrite: bool) -> (Vec<String>, Vec<String>, Vec<(u32, bool)>) {
    use crate::game::native_guest_turn_tests::with_guests;
    super::super::guest_body::NO_MERGE.set(!merge);
    super::super::guest_body::SKIP.set(skip);
    let heard = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "turn-listener",
        Box::new(Listener(heard.clone(), rewrite)),
    )]
    .into();
    let log = std::sync::Mutex::new(Vec::new());
    let trace = std::sync::Mutex::new(Vec::new());
    mod_platform::testing::with_mods(mods, || {
        with_guests(|game| {
            for conn in [12, 13] {
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .update_guest(conn, |g| {
                        g.loaded = true;
                        g.placed = true;
                        g.load_timeout = 0;
                    })
                    .unwrap();
            }
            game.set_allow_commands(true);
            assert!(game.run_command("scoreboard objectives add fd food"));
            for tick in 0..30 {
                if tick >= 3 {
                    game.as_guest(13, |g| {
                        g.player.as_mut().unwrap().health.food.exhaustion = 5.0
                    });
                }
                if tick % 7 == 3 {
                    game.as_guest(13, |g| g.player.as_mut().unwrap().health.food.food -= 2);
                }
                super::super::guest_body::TRACE.with_borrow_mut(Vec::clear);
                game.tick_guests();
                trace
                    .lock()
                    .unwrap()
                    .extend(super::super::guest_body::TRACE.with_borrow(|t| t.clone()));
                let host = game.lan.as_ref().unwrap();
                let line: Vec<String> = host
                    .hub
                    .guests
                    .iter()
                    .map(|g| {
                        format!(
                            "{} {:?} hp={:08x} food={} sat={:08x} ground={}",
                            g.conn,
                            g.position.map(f64::to_bits),
                            g.health.to_bits(),
                            g.food,
                            g.saturation.to_bits(),
                            g.on_ground
                        )
                    })
                    .collect();
                log.lock().unwrap().push(format!(
                    "{} | scores {:?}",
                    line.join(" | "),
                    game.score.board.scores
                ));
            }
        });
    });
    super::super::guest_body::NO_MERGE.set(false);
    super::super::guest_body::SKIP.set(0);
    let heard = heard.lock().unwrap().clone();
    (
        log.into_inner().unwrap(),
        heard,
        trace.into_inner().unwrap(),
    )
}

#[test]
#[ignore = "daily"]
fn a_mod_listening_to_the_guests_hooks_hears_the_same_with_the_merge() {
    let mut all = 0u64;
    for (i, _) in conditions() {
        all |= 1 << i;
    }
    for rewrite in [false, true] {
        let (old_log, old_heard, old_trace) = listened(false, 0, rewrite);
        let (new_log, new_heard, new_trace) = listened(true, 0, rewrite);
        assert!(old_trace.iter().all(|t| !t.1));
        assert_eq!(
            old_heard, new_heard,
            "rewrite {rewrite}: what the hooks heard, in order"
        );
        if let Some(d) = difference(&old_log, &new_log) {
            panic!("rewrite {rewrite}: {d}");
        }
        let count = |what: &str| new_heard.iter().filter(|h| h.starts_with(what)).count();
        assert!(count("pre") >= 30 && count("post") >= 30, "{new_heard:?}");
        assert!(count("food") >= 3, "the food hook was heard: {new_heard:?}");
        let (forced_log, forced_heard, _) = listened(true, all, rewrite);
        eprintln!(
            "listener rewrite={rewrite}: merged {} apart {}; with no gate at all: hooks {} logs {}",
            new_trace.iter().filter(|t| t.1).count(),
            new_trace.iter().filter(|t| !t.1).count(),
            if forced_heard == old_heard {
                "same"
            } else {
                "DIFFER"
            },
            difference(&old_log, &forced_log).map_or("same".to_owned(), |d| format!(
                "DIFFER {}",
                d.lines().next().unwrap_or("")
            )),
        );
    }
}

/// The scenarios with the merge forced (no condition at all): which differ from the old path.
#[test]
#[ignore = "daily"]
fn forced_merge_probe() {
    // A probe of about 6 minutes that asserts nothing: run with PROBE=1.
    if std::env::var_os("PROBE").is_none() {
        return;
    }
    let only = std::env::var("CASE").unwrap_or_default();
    let mut all = 0u64;
    for (i, _) in conditions() {
        all |= 1 << i;
    }
    // REMOVE=15,40: only these conditions are taken away; KEEP=12,16: all but these are.
    if let Ok(list) = std::env::var("REMOVE") {
        all = list
            .split(',')
            .filter_map(|n| n.parse::<u32>().ok())
            .fold(0, |m, i| m | 1 << i);
    }
    if let Ok(list) = std::env::var("KEEP") {
        for i in list.split(',').filter_map(|n| n.parse::<u32>().ok()) {
            all &= !(1u64 << i);
        }
    }
    // The sensitivity control: the merged turn leaves its tail (bit 62) or its sculk step (bit 61)
    // out. A test that cannot see that cannot see a wrong merge either.
    let extra = std::env::var("OMIT").map_or(0, |v| match v.as_str() {
        "tail" => 1u64 << 62,
        "sculk" => 1 << 61,
        _ => 0,
    });
    for case in CASES.iter().filter(|c| c.name.contains(&only)) {
        let old = play(case, false, 0);
        let forced = play(case, true, all | extra);
        let verdict = match difference(&old.log, &forced.log) {
            Some(d) => format!("DIFFERS: {}", d.lines().next().unwrap_or("")),
            None => "same".to_owned(),
        };
        let shown: String = forced
            .trace
            .iter()
            .map(|t| match t.iter().find(|e| e.0 == 2) {
                Some(&(_, true)) => 'M',
                Some(&(_, false)) => 'a',
                None => '-',
            })
            .collect();
        let hp: Vec<String> = forced
            .hp
            .iter()
            .map(|h| format!("{:.0}@{:.0}", h.0, h.1))
            .collect();
        let seen = |k: usize| forced.states.iter().filter(|s| s[k]).count();
        eprintln!(
            "   ticks riding {} sleeping {} gliding {} sprinting {}",
            seen(0),
            seen(1),
            seen(2),
            seen(3)
        );
        eprintln!(
            "FORCED {:44} {verdict}\n   path {shown}\n   hp   {}",
            case.name,
            hp.join(",")
        );
    }
    if only.is_empty() || "hook".contains(&only) {
        let (old, _, _) = listened(false, 0, true);
        let (forced, _, _) = listened(true, all, true);
        eprintln!(
            "FORCED a mod that keeps the food                  {}",
            difference(&old, &forced).map_or("same".to_owned(), |d| format!(
                "DIFFERS: {}",
                d.lines().next().unwrap_or("")
            ))
        );
    }
}

/// What the gate is for, shown where a scenario shows it: with the merge forced (no condition)
/// a player a totem saves, and a mod that keeps the food level, see the tail's state differ from
/// the old path; the food condition alone, and any one of health, events and velocity, stop each
/// of them; and the digest sees the tail or the sculk step missing, so equality means something.
#[test]
#[ignore = "daily"]
fn the_gate_is_needed_where_it_matters() {
    let bit = |list: &[u32]| list.iter().fold(0u64, |m, i| m | 1 << i);
    let mut all = 0u64;
    for (i, _) in conditions() {
        all |= 1 << i;
    }
    let totem = CASES
        .iter()
        .find(|c| c.name == "lava and a totem, 3 health left")
        .unwrap();
    let sculk = CASES.iter().find(|c| c.name.starts_with("sculk")).unwrap();
    let differs = |case: &Case, skip: u64| {
        let old = play(case, false, 0);
        difference(&old.log, &play(case, true, skip).log).is_some()
    };
    // The totem: health (12) and the velocity a hurt marks (41) each stop it; neither, and it shows.
    assert!(
        differs(totem, all),
        "no gate: the tail sees the health before the totem"
    );
    assert!(!differs(totem, 0), "the gate");
    for kept in [12, 41] {
        assert!(
            !differs(totem, all & !bit(&[kept])),
            "condition {kept} alone stops it"
        );
    }
    assert!(
        differs(totem, bit(&[12, 15, 40, 41])),
        "the four taken away: it shows"
    );
    // The food a mod keeps.
    let (old, _, _) = listened(false, 0, true);
    let (merged, _, _) = listened(true, 0, true);
    let (forced, _, _) = listened(true, all, true);
    let (no_food, _, _) = listened(true, bit(&[16]), true);
    assert!(difference(&old, &merged).is_none(), "the gate");
    assert!(
        difference(&old, &forced).is_some(),
        "no gate: the scoreboard sees the food before the mod"
    );
    assert!(
        difference(&old, &no_food).is_some(),
        "the food condition alone is needed"
    );
    // The digest sees a missing tail and a missing sculk step.
    let quiet = CASES.iter().find(|c| c.name == "quiet").unwrap();
    assert!(differs(quiet, bit(&[62])), "no tail in the merged turn");
    assert!(
        differs(sculk, bit(&[61])),
        "no sculk step in the merged turn"
    );
}
