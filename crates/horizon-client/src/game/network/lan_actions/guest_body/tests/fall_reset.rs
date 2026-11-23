//! `handlePlayerPositionChange` and block entity ticks update the guest's fall.
use super::guest;
use crate::game::Game;
use horizon_client::lan::guest::InboxItem;
use horizon_client::{
    advancements::Tree,
    chunk::Chunk,
    lan::{NetConfig, hub::HubConfig},
};
use horizon_core::position::ChunkPos;
use horizon_protocol::packets::{Empty, play, play::Serverbound as S};
use std::sync::Arc;

const NAME: &str = "GuestBodyTest";
/// `fall_from_height` with no condition, then with a fall of at most 1 block, and of 5.
const CRITERIA: [&str; 3] = ["any", "short", "five"];

/// A server with one survival guest in the air at 12.5 110 12.5, over a stone floor at y 99.
fn scene(resources: &mut Option<Arc<crate::game::Resources>>, with_advancements: bool) -> Game<'_> {
    let mut g = crate::game::game_tests::tests::server_game(resources);
    g.build_registries();
    g.set_allow_commands(true);
    if with_advancements {
        install_fall_advancements(&mut g);
    }
    let stone = g.pack.assets.registry.default_state("minecraft:stone");
    let mut chunk = Chunk::empty(0, 0);
    for x in 0..16 {
        for z in 0..16 {
            chunk.set(x, 99, z, stone.unwrap().0);
        }
    }
    g.world.chunks.insert((0, 0), Arc::new(chunk));
    g.block_ticks.add_empty_chunk(ChunkPos::new(0, 0)).unwrap();
    assert!(g.run_command("forceload add 0 0"));
    assert!(g.run_command("tick freeze"));
    g.lan_host(
        crate::game::lan::LanSettings {
            port: 0,
            guest_commands: true,
            force_game_mode: false,
        },
        NetConfig {
            listen: false,
            announce: false,
            ..NetConfig::loopback(0, "guest-fall-reset".into(), 2)
        },
        HubConfig {
            default_game_mode: 0,
            guest_commands: true,
            force_game_mode: false,
            hardcore: false,
            view_distance: 2,
            simulation_distance: 2,
            dimension: "minecraft:overworld".into(),
            allow_flight: false,
            seed: 0,
            max_players: 2,
            integrated: false,
            levels: Default::default(),
            level_seeds: Default::default(),
        },
    )
    .unwrap();
    let mut guest = guest(1);
    guest.position = [12.5, 110.0, 12.5];
    guest.placed = true;
    guest.loaded = true;
    guest.dimension = "minecraft:overworld".into();
    guest.game_mode = 0;
    guest.on_ground = false;
    guest.load_timeout = 0;
    g.lan.as_mut().unwrap().hub.guests.push(guest);
    g.sync_tick_chunks();
    g.ticked_entities_through = g.next_entity;
    g
}

fn install_fall_advancements(g: &mut Game<'_>) {
    let scratch = std::env::current_dir().unwrap().join("guest-fall-reset");
    assert!(!scratch.exists());
    let folder = scratch.join("data/fall/advancement");
    std::fs::create_dir_all(&folder).unwrap();
    g.world_root = scratch.clone();
    for (name, distance) in CRITERIA.into_iter().zip([
        serde_json::Value::Null,
        serde_json::json!({"max": 1.0}),
        serde_json::json!({"min": 4.9, "max": 5.1}),
    ]) {
        let mut criterion = serde_json::json!({"trigger": "minecraft:fall_from_height"});
        if !distance.is_null() {
            criterion["conditions"] = serde_json::json!({"distance": {"y": distance}});
        }
        let advancement = serde_json::json!({"criteria": {name: criterion}});
        std::fs::write(folder.join(format!("{name}.json")), advancement.to_string()).unwrap();
    }
    g.adv.tree = Arc::new(Tree::load(&scratch.join("data")));
    assert_eq!(g.adv.tree.list.len(), CRITERIA.len());
}

/// Moves as sent by a client, one per client tick.
fn queue_moves(g: &mut Game<'_>, x: f64, heights: &[f64]) {
    g.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
        guest
            .inbox
            .push_back(InboxItem::Packet(S::ClientTickEnd(Empty), None));
        for &y in heights {
            let to = play::MovePlayerPos {
                x,
                y,
                z: 12.5,
                flags: 0,
            };
            guest
                .inbox
                .push_back(InboxItem::Packet(S::MovePlayerPos(to), None));
            guest
                .inbox
                .push_back(InboxItem::Packet(S::ClientTickEnd(Empty), None));
        }
    });
}

fn tick_with_moves(g: &mut Game<'_>, heights: &[f64]) {
    queue_moves(g, 12.5, heights);
    g.tick_lan_packets();
    let guest = &g.lan.as_ref().unwrap().hub.guests[0];
    assert_eq!(guest.moves.len(), heights.len(), "every move is accepted");
    assert_eq!(guest.position[1], *heights.last().unwrap());
    g.tick_lan(false);
}

/// The guest's fall distance, where its tracked fall began, and which criteria it has.
fn fall(g: &Game<'_>) -> (f64, Option<[f64; 3]>, [bool; 3]) {
    let host = g.lan.as_ref().unwrap();
    let guest = &host.hub.guests[0];
    let done = CRITERIA.map(|name| {
        let i = g.adv.tree.find_id(&format!("fall:{name}")).unwrap();
        guest.advancements.done(&g.adv.tree.list[i])
    });
    (
        guest.motion.as_ref().unwrap().fall_distance,
        host.play.get(&1).unwrap().marks.fall,
        done,
    )
}

#[test]
#[ignore = "daily"]
fn a_guest_move_that_goes_up_ends_its_fall() {
    let mut resources = None;
    let mut g = scene(&mut resources, true);
    // One block down: a fall of 1, tracked from where the tick left the guest.
    tick_with_moves(&mut g, &[109.0]);
    assert_eq!(fall(&g), (1.0, Some([12.5, 109.0, 12.5]), [false; 3]));
    // Half a block up: no fall left, and the fall of half a block is awarded.
    tick_with_moves(&mut g, &[109.5]);
    assert_eq!(fall(&g), (0.0, None, [true, true, false]));

    for name in ["any", "short"] {
        assert!(g.run_command(&format!("advancement revoke {NAME} only fall:{name}")));
    }
    tick_with_moves(&mut g, &[108.5]);
    assert_eq!(fall(&g), (1.0, Some([12.5, 108.5, 12.5]), [false; 3]));
    // Three moves in one server tick: down to 104, up to 104.5, down to 103.5. The move up
    // ends the fall where it is, 4 blocks below its start, not at the tick's end, 5 below.
    // The last move starts a new fall of 1.
    tick_with_moves(&mut g, &[104.0, 104.5, 103.5]);
    assert_eq!(
        fall(&g),
        (1.0, Some([12.5, 103.5, 12.5]), [true, false, false])
    );

    // A player that dies later in the tick was alive at its move up, so the fall still counts:
    // half a heart left, and a block at its head to suffocate in once it has moved.
    assert!(g.run_command(&format!("advancement revoke {NAME} only fall:any")));
    assert!(g.run_command("gamerule natural_health_regeneration false"));
    let health = g.lan.as_ref().unwrap().hub.guests[0].health;
    assert!(g.run_command(&format!("damage {NAME} {} minecraft:generic", health - 0.5)));
    // The damage clock runs out, so the next damage counts.
    for _ in 0..20 {
        g.tick_lan(false);
    }
    assert!(g.run_command("setblock 12 105 12 minecraft:stone"));
    tick_with_moves(&mut g, &[104.0]);
    let guest = &g.lan.as_ref().unwrap().hub.guests[0];
    assert!(
        guest.health <= 0.0,
        "the guest suffocated: {}",
        guest.health
    );
    // Half a block up from where the last fall began: any fall, and a short one.
    assert_eq!(fall(&g).2, [true, true, false]);
}

#[test]
#[ignore = "daily"]
fn a_geyser_caps_only_the_slow_fall_of_a_guest_in_its_column() {
    let mut resources = None;
    let mut trace = |sulfur: &str| {
        let mut g = scene(&mut resources, false);
        // Lava beneath the sulfur; four glass sides contain one water source.
        // The active column spans y=102 through y<108.
        for command in [
            "setblock 12 100 12 minecraft:lava",
            "setblock 11 102 12 minecraft:glass",
            "setblock 13 102 12 minecraft:glass",
            "setblock 12 102 11 minecraft:glass",
            "setblock 12 102 13 minecraft:glass",
            "setblock 12 102 12 minecraft:water",
        ] {
            assert!(g.run_command(command), "{command}");
        }
        assert!(g.run_command(&format!("setblock 12 101 12 {sulfur}")));
        [
            (12.5, 109.25),
            (12.5, 108.0), // touches the top without overlapping
            (12.5, 107.5), // exactly -0.5 is still too fast
            (12.5, 107.25),
            (12.5, 107.0),  // no upward packet is needed
            (10.5, 106.75), // outside the column at the same height
            (10.5, 106.0),
        ]
        .map(|(x, y)| {
            queue_moves(&mut g, x, &[y]);
            assert!(g.run_command("tick step 1"));
            g.tick_server(false).unwrap();
            let guest = &g.lan.as_ref().unwrap().hub.guests[0];
            assert_eq!(
                guest.position,
                [x, y, 12.5],
                "the server accepted this move"
            );
            let body = guest.motion.as_ref().unwrap();
            (body.fall_distance, body.velocity)
        })
    };
    let active = trace("minecraft:potent_sulfur[potent_sulfur_state=continuous]");
    let no_geyser = trace("minecraft:smooth_stone");
    assert_eq!(
        active.map(|(fall, _)| fall),
        [0.75, 2.0, 2.5, 1.0, 1.0, 1.25, 2.0]
    );
    assert_eq!(
        no_geyser.map(|(fall, _)| fall),
        [0.75, 2.0, 2.5, 2.75, 3.0, 3.25, 4.0]
    );
    assert_eq!(
        active.map(|(_, velocity)| velocity),
        no_geyser.map(|(_, velocity)| velocity),
        "the server does not add a launch impulse"
    );
}
