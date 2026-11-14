//! `BucketItem.use` asks `Level.mayInteract` before it touches the world
//! (`BucketItem.java:59`), so on a dedicated server a guest who is not an
//! operator neither empties nor fills a bucket inside the spawn protection
//! radius, while an operator does both, and a non-operator outside it does.
use super::*;
use crate::game::lan::LanHost;
use horizon_client::chunk::Chunk;
use horizon_client::lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig};
use horizon_core::position::ChunkPos;
use horizon_protocol::packets::{
    configuration::ClientInformation,
    play::{Serverbound, UseItem},
};
use std::sync::{Arc, atomic::AtomicI32, mpsc};

const CONN: u32 = 2;
const GUEST_UUID: u128 = 424;
/// The stone the guest stands on, and the air above it a pour fills.
const FLOOR: [i32; 3] = [8, 199, 8];
const POURED: [i32; 3] = [8, 200, 8];
/// The air above a stone block 17 blocks east of the spawn: past the radius.
const OUTSIDE: [i32; 3] = [25, 200, 8];
/// A water source in the same protected area, and the air its pickup leaves.
const SOURCE: [i32; 3] = [10, 199, 10];

/// `ServerboundUseItemPacket` from a player looking straight down.
fn use_item(sequence: i32) -> Serverbound {
    Serverbound::UseItem(UseItem {
        hand: 0,
        yaw: 0.0,
        pitch: 90.0,
        sequence,
    })
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_guest_without_build_rights_neither_empties_nor_fills_a_bucket_under_spawn_protection() {
    let mut game = crate::game::native_machines::tests::server_game();
    game.build_registries();
    game.set_allow_commands(true);
    game.block_ticks
        .add_empty_chunk(ChunkPos::new(0, 0))
        .unwrap();
    // The second chunk holds the floor outside the radius.
    game.world
        .chunks
        .insert((1, 0), Arc::new(Chunk::empty(1, 0)));
    assert!(game.run_command("fill 0 199 0 31 199 15 stone"));
    assert!(game.run_command("setblock 10 199 10 water"));
    // The world spawn sits inside what the guest aims at, and the server
    // protects it (`DedicatedServer.isUnderSpawnProtection`).
    game.level.world_spawn = Some(FLOOR);
    game.lan_host(
        crate::game::lan::LanSettings {
            port: 0,
            guest_commands: true,
            force_game_mode: false,
        },
        NetConfig {
            announce: false,
            ..NetConfig::loopback(0, "bucket-spawn-protection-test".into(), 2)
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
        },
    )
    .unwrap();
    let (tx, _rx) = mpsc::channel();
    let mut guest = Guest::new(
        Joined {
            conn: CONN,
            uuid: GUEST_UUID,
            name: "BucketGuest".into(),
            properties: Vec::new(),
            info: ClientInformation::new(2),
            brand: "test".into(),
            sender: PlayerSender::new(tx, Arc::default()),
            latency: Arc::new(AtomicI32::new(0)),
            address: "127.0.0.1:25565".parse().unwrap(),
            expected: false,
            configured: Default::default(),
        },
        CONN as i32,
        [8.5, 200.0, 8.5],
        2,
    );
    guest.dimension = "minecraft:overworld".into();
    guest.placed = true;
    guest.loaded = true;
    guest.selected = 0;
    let mut host = game.lan.take().unwrap();
    host.hub.guests.push(guest);
    // `spawn-protection` in server.properties, and someone else is an
    // operator: `PlayerList.getOps()` is not empty, so the radius holds.
    host.hub.spawn_protection = 16;
    host.hub.ops.insert(900, 4);

    let registry = game.pack.assets.registry;
    let air = registry.default_state("minecraft:air").unwrap().0;
    let water = registry.default_state("minecraft:water").unwrap().0;
    let state = |game: &Game<'_>, pos: [i32; 3]| game.world.block(pos[0], pos[1], pos[2]).0;
    let held = |host: &LanHost| {
        host.hub.guests[0].inventory[0]
            .as_ref()
            .map(|s| s.item.to_string())
    };
    let give = |host: &mut LanHost, item: &str| {
        host.hub.guests[0].inventory[0] = Some(ItemStack::new(item, 1));
    };
    let move_to = |host: &mut LanHost, at: [f64; 3]| {
        host.hub.guests[0].position = at;
        host.hub.guests[0].previous_position = at;
    };

    // Emptying a water bucket on the floor: the non-operator is refused, and
    // keeps the full bucket.
    give(&mut host, "minecraft:water_bucket");
    game.lan_guest_packet(&mut host, CONN, use_item(1));
    assert_eq!(
        state(&game, POURED),
        air,
        "a non-operator's water is refused"
    );
    assert_eq!(
        held(&host).as_deref(),
        Some("minecraft:water_bucket"),
        "and the refused bucket is not emptied"
    );
    // The same non-operator 17 blocks east of the spawn, past the radius,
    // pours it: `isUnderSpawnProtection` measures the distance to the spawn.
    move_to(&mut host, [25.5, 200.0, 8.5]);
    game.lan_guest_packet(&mut host, CONN, use_item(2));
    assert_eq!(
        state(&game, OUTSIDE),
        water,
        "outside the radius a non-operator pours"
    );
    assert_eq!(
        held(&host).as_deref(),
        Some("minecraft:bucket"),
        "and gets the empty bucket"
    );
    // The same click from an operator pours it inside the radius.
    move_to(&mut host, [8.5, 200.0, 8.5]);
    give(&mut host, "minecraft:water_bucket");
    host.hub.ops.insert(GUEST_UUID, 4);
    game.lan_guest_packet(&mut host, CONN, use_item(3));
    assert_eq!(state(&game, POURED), water, "an operator still pours");
    assert_eq!(
        held(&host).as_deref(),
        Some("minecraft:bucket"),
        "and gets the empty bucket"
    );
    // Filling from a water source in the protected area, from beside it.
    host.hub.ops.remove(&GUEST_UUID);
    move_to(&mut host, [10.5, 200.0, 10.5]);
    give(&mut host, "minecraft:bucket");
    game.lan_guest_packet(&mut host, CONN, use_item(4));
    assert_eq!(
        state(&game, SOURCE),
        water,
        "a non-operator's fill is refused"
    );
    assert_eq!(
        held(&host).as_deref(),
        Some("minecraft:bucket"),
        "and the refused bucket stays empty"
    );
    // The operator fills it.
    host.hub.ops.insert(GUEST_UUID, 4);
    game.lan_guest_packet(&mut host, CONN, use_item(5));
    assert_eq!(state(&game, SOURCE), air, "an operator still fills");
    assert_eq!(
        held(&host).as_deref(),
        Some("minecraft:water_bucket"),
        "and gets the full bucket"
    );
}
