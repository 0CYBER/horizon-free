//! Spawn protection is the world spawn's, in the spawn's own dimension
//! (`DedicatedServer.isUnderSpawnProtection` compares `level.dimension()` with
//! `ServerLevel.getRespawnData()`, `LevelData.RespawnData`'s `GlobalPos`), and
//! it holds a player's attack off the decorations too
//! (`BlockAttachedEntity.skipAttackInteraction`,
//! `ArmorStand.skipAttackInteraction`: `!level.mayInteract(player, pos)`).
use super::*;
use crate::game::lan::LanHost;
use horizon_client::chunk::Chunk;
use horizon_client::lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig};
use horizon_core::position::ChunkPos;
use horizon_protocol::packets::{
    configuration::ClientInformation,
    play::{self, Serverbound},
};
use std::sync::{Arc, atomic::AtomicI32, mpsc};

const CONN: u32 = 2;
const GUEST_UUID: u128 = 424;
/// The guest's entity id: clear of the decorations' (`summon_with_nbt` numbers
/// from one) and of `CONN`.
const GUEST_ENTITY: i32 = 700;
/// The world spawn, inside the radius of every decoration below.
const SPAWN: [i32; 3] = [8, 200, 8];
/// `spawn-protection` in server.properties: the radius the server holds a
/// non-operator back in.
const RADIUS: i32 = 16;

fn attack(id: i32) -> Serverbound {
    Serverbound::Attack(play::Attack { id })
}

/// A dedicated server with a stone floor, a wall along z = 7 and a creative
/// guest that `spawn-protection` holds back.
fn server() -> (Game<'static>, LanHost) {
    let mut game = crate::game::native_machines::tests::server_game();
    game.build_registries();
    game.set_allow_commands(true);
    game.block_ticks
        .add_empty_chunk(ChunkPos::new(0, 0))
        .unwrap();
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    assert!(game.run_command("fill 0 199 0 15 199 15 stone"));
    // The decorations hang on this wall.
    assert!(game.run_command("fill 5 200 7 11 201 7 stone"));
    game.lan_host(
        crate::game::lan::LanSettings {
            port: 0,
            guest_commands: true,
            force_game_mode: false,
        },
        NetConfig {
            announce: false,
            ..NetConfig::loopback(0, "spawn-protection-test".into(), 2)
        },
        HubConfig {
            default_game_mode: 1,
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
        },
    )
    .unwrap();
    let (tx, _rx) = mpsc::channel();
    let mut guest = Guest::new(
        Joined {
            conn: CONN,
            uuid: GUEST_UUID,
            name: "SpawnGuest".into(),
            properties: Vec::new(),
            info: ClientInformation::new(2),
            brand: "test".into(),
            sender: PlayerSender::new(tx, Arc::default()),
            latency: Arc::new(AtomicI32::new(0)),
            address: "127.0.0.1:25565".parse().unwrap(),
            expected: false,
            configured: Default::default(),
        },
        GUEST_ENTITY,
        [8.5, 200.0, 10.5],
        2,
    );
    guest.dimension = "minecraft:overworld".into();
    guest.placed = true;
    guest.loaded = true;
    // A creative guest: one hit breaks a painting, a frame or a stand.
    guest.game_mode = 1;
    let mut host = game.lan.take().unwrap();
    host.hub.guests.push(guest);
    // Someone else is an operator, so `PlayerList.getOps()` is not empty and
    // the radius holds; the guest is not one.
    host.hub.ops.insert(900, 4);
    host.hub.spawn_protection = RADIUS;
    (game, host)
}

/// Puts the guest at `at` and has it punch entity `id`.
fn punch(game: &mut Game<'_>, host: &mut LanHost, at: [f64; 3], id: i32) {
    host.hub.guests[0].position = at;
    host.hub.guests[0].previous_position = at;
    game.lan_guest_packet(host, CONN, attack(id));
}

/// `/setworldspawn` stores `RespawnData.of(level.dimension(), pos, yaw, pitch)`
/// with the block, and `DedicatedServer.isUnderSpawnProtection` compares the
/// level it is asked about with that dimension: a spawn set in the Nether
/// protects the Nether's blocks at those coordinates, not the Overworld's.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn the_protected_area_follows_the_world_spawns_dimension() {
    let mut game = crate::game::native_machines::tests::server_game();
    game.build_registries();
    game.set_allow_commands(true);
    assert!(game.run_command("setworldspawn 8 200 8"));
    assert!(
        game.spawn_protects(RADIUS, "minecraft:overworld", SPAWN),
        "the Overworld block at the spawn is protected"
    );
    assert!(
        !game.spawn_protects(RADIUS, "minecraft:the_nether", SPAWN),
        "an Overworld spawn protects nothing in the Nether"
    );
    // The spawn moved into the Nether: `RespawnData` carries the dimension,
    // so the same coordinates are protected there and nowhere else.
    assert!(
        game.run_command("execute in minecraft:the_nether run setworldspawn 8 200 8"),
        "setworldspawn runs in the Nether"
    );
    assert!(
        game.spawn_protects(RADIUS, "minecraft:the_nether", SPAWN),
        "a Nether spawn protects the Nether"
    );
    assert!(
        !game.spawn_protects(RADIUS, "minecraft:overworld", SPAWN),
        "and no longer the Overworld's block at those coordinates"
    );
    // `spawn.dimension` is in level.dat (`GlobalPos.MAP_CODEC`): the level
    // that comes back from the file protects the Nether too.
    let dir = std::env::temp_dir().join(format!("horizon-spawn-dimension-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    game.level.write(&dir).unwrap();
    game.level = crate::save::Level::read(&dir).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        game.spawn_protects(RADIUS, "minecraft:the_nether", SPAWN),
        "the saved spawn keeps its dimension"
    );
    assert!(
        !game.spawn_protects(RADIUS, "minecraft:overworld", SPAWN),
        "and the Overworld has none at those coordinates"
    );
}

/// A painting, an item frame and an armour stand inside the spawn protection
/// radius survive a guest's punch, and go when the radius is off: the
/// decorations ask `mayInteract` first.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_guest_cannot_knock_a_decoration_off_inside_spawn_protection() {
    let (mut game, mut host) = server();
    game.level.world_spawn = Some(SPAWN);
    assert!(
        game.summon_with_nbt(
            "minecraft:painting",
            [6.5, 200.5, 8.5],
            "{variant:\"minecraft:kebab\",facing:0b}",
        )
        .unwrap()
        .is_some(),
        "the painting joins"
    );
    assert!(
        game.summon_with_nbt("minecraft:item_frame", [8.5, 200.5, 8.5], "{Facing:3b}")
            .unwrap()
            .is_some(),
        "the frame joins"
    );
    assert!(
        game.summon_with_nbt("minecraft:armor_stand", [10.5, 200.0, 9.5], "{}")
            .unwrap()
            .is_some(),
        "the stand joins"
    );
    let punches = [
        // Where the guest stands to punch the painting, the frame and the
        // stand: two blocks from each.
        ([6.5, 200.0, 10.5], game.paintings[0].id as i32),
        ([8.5, 200.0, 10.5], game.item_frames[0].id as i32),
        ([10.5, 200.0, 11.5], game.armor_stands[0].id as i32),
    ];

    // Inside the radius: `mayInteract` refuses the hit, so nothing is knocked
    // off and nothing drops.
    for (at, id) in punches {
        punch(&mut game, &mut host, at, id);
    }
    assert_eq!(game.paintings.len(), 1, "the painting stays on the wall");
    assert_eq!(game.item_frames.len(), 1, "the frame stays on the wall");
    assert_eq!(game.armor_stands.len(), 1, "the stand stays put");
    assert!(game.dropped.is_empty(), "nothing was knocked off");

    // `spawn-protection=0`: the same punches knock all three off.
    host.hub.spawn_protection = 0;
    for (at, id) in punches {
        punch(&mut game, &mut host, at, id);
    }
    assert!(game.paintings.is_empty(), "the painting breaks unprotected");
    assert!(game.item_frames.is_empty(), "the frame breaks unprotected");
    assert!(
        game.armor_stands.is_empty(),
        "the creative guest breaks the stand unprotected"
    );
}
