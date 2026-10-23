//! Serve a small flat world on the LAN server without the game, for joining
//! with a real client while developing the protocol:
//! `cargo run --release --example lan_flat -- RESOURCES PORT`.
use horizon_client::{
    assets::Assets,
    blocks::BlockFacts,
    chunk::{Chunk, MIN_Y},
    lan::{
        Lan,
        chunks::ChunkEncoder,
        data::SyncData,
        hub::{HostEvent, HostState, Hub, HubConfig, TickCtx},
    },
    world::World,
};
use horizon_protocol::packets::play::Serverbound as S;
use std::{path::PathBuf, sync::Arc, time::Duration};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let root = PathBuf::from(args.next().ok_or("RESOURCES PORT")?);
    let port: u16 = args.next().ok_or("port")?.parse()?;
    let assets = Assets::load(&root)?;
    let facts = BlockFacts::load(&root, assets.registry)?;
    let state = |name: &str| assets.registry.default_state(name).map_or(0, |s| s.0);
    let sync = Arc::new(SyncData::load(&root)?);
    let biomes: Vec<String> = sync
        .dynamic
        .registries
        .get("minecraft:worldgen/biome")
        .map(|v| v.iter().map(|(n, _)| n.clone()).collect())
        .unwrap_or_default();
    let encoder = ChunkEncoder::new(
        facts
            .states
            .iter()
            .map(|s| (s.name.clone(), s.fluid_level > 0 || s.waterlogged)),
        &sync.builtin,
        &biomes,
        &biomes,
    );
    let (stone, dirt, grass) = (
        state("minecraft:stone"),
        state("minecraft:dirt"),
        state("minecraft:grass_block"),
    );
    let mut world = World::new(facts.air, 0);
    for cz in -8..=8 {
        for cx in -8..=8 {
            let mut chunk = Chunk::empty(cx, cz);
            for z in 0..16 {
                for x in 0..16 {
                    for y in MIN_Y..60 {
                        let v = if y == 59 {
                            grass
                        } else if y > 55 {
                            dirt
                        } else {
                            stone
                        };
                        chunk.set(x, y, z, v);
                    }
                }
            }
            chunk.lit = true;
            world.chunks.insert((cx, cz), Arc::new(chunk));
        }
    }
    let lan = Lan::start(sync.clone(), port, "Flat - Example".into(), 8)?;
    println!("listening on {}", lan.port);
    let mut hub = Hub::new(
        lan,
        sync,
        encoder,
        HubConfig {
            default_game_mode: 1,
            guest_commands: true,
            force_game_mode: false,
            hardcore: false,
            view_distance: 8,
            simulation_distance: 8,
            dimension: "minecraft:overworld".into(),
            seed: 0,
            max_players: 8,
            dedicated: false,
            integrated: false,
        },
    );
    let mut next_entity = 100u32;
    let host = HostState {
        uuid: 42,
        name: "Host".into(),
        entity_id: 1,
        position: [0.5, 60.0, 0.5],
        yaw: 0.0,
        pitch: 0.0,
        head_yaw: 0.0,
        on_ground: true,
        game_mode: 1,
        sneaking: false,
        sprinting: false,
        equipment: Default::default(),
        health: 20.0,
        attributes: Vec::new(),
        bed: None,
        living_flags: 0,
    };
    loop {
        world.game_time += 1;
        world.day_time += 1;
        let mut ctx = TickCtx {
            world: &mut world,
            host: Some(&host),
            entities: &[],
            next_entity: &mut next_entity,
            spawn: [8.5, 60.0, 8.5],
            difficulty: 2,
            difficulty_locked: false,
            clock_rate: 1.0,
            tick_rate: 20.0,
            tick_frozen: false,
            tick_steps: (0, 0),
            collision: None,
            border: &horizon_client::worldborder::WorldBorder::default(),
        };
        for event in hub.tick(&mut ctx) {
            match event {
                HostEvent::Joined(conn) => hub.place_new_player(conn, &mut ctx),
                HostEvent::Packet(_, packet) => {
                    if std::env::var_os("HORIZON_NET_DEBUG").is_some()
                        && !matches!(*packet, S::ClientTickEnd(_))
                    {
                        eprintln!("[lan] {packet:?}");
                    }
                }
                HostEvent::Chat { line, .. } | HostEvent::Warn(line) | HostEvent::Error(line) => {
                    println!("{line}")
                }
                HostEvent::Left(_) => {}
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}
