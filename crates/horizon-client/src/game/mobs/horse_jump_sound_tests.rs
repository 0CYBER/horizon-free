//! `AbstractHorse.handleStartJump` is where vanilla plays the riding jump
//! sound: START_RIDING_JUMP (`ServerGamePacketListenerImpl.handlePlayerCommand`)
//! calls `handleStartJump`, which rears the horse and calls `playJumpSound`
//! (0.4, 1.0) once. The server's own simulation of a guest's jump bar must not
//! play it again (#16360: a rider heard it twice).
use crate::game::*;
use horizon_client::{
    chunk::Chunk,
    lan::{Joined, NetConfig, Out, PlayerSender, guest::Guest, hub::HubConfig},
    motion::PlayerMotion,
};
use horizon_protocol::packets::{
    configuration::ClientInformation,
    play::{self, Serverbound as S},
};
use std::sync::{Arc, atomic::AtomicI32, mpsc};

const GUEST: &str = "JumpGuest";
const JUMP: &str = "minecraft:entity.horse.jump";

/// A host on a stone floor with one placed guest (conn 2) riding a saddled,
/// tamed horse beside it. The horse is not silent: `Entity.playSound` plays
/// nothing for a silent mob, so a silent horse would make no jump sound at
/// all (`AbstractHorse.playJumpSound`).
fn game() -> (Game<'static>, mpsc::Receiver<Out>) {
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
        execution_role::ExecutionRole::IntegratedServer,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.game_dir =
        Some(std::env::temp_dir().join(format!("horizon-horse-jump-unit-{}", std::process::id())));
    options.execution_role = execution_role::ExecutionRole::IntegratedServer;
    let res: &'static _ = Box::leak(Box::new(loaded.resources));
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
        game.load_ticking_neighbourhood(x, 0);
    }
    assert!(game.run_command("fill 0 99 0 31 99 15 stone"));
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(4.5, 100.0, 6.5)).unwrap());
    assert!(game.run_command("summon minecraft:horse 4.5 100 4.5 {Tame:1b}"));
    let horse = game.mobs[0].id;
    game.mobs[0].extra.set_saddled(true);
    game.lan_host(
        crate::game::lan::LanSettings {
            port: 0,
            guest_commands: true,
            force_game_mode: false,
        },
        NetConfig {
            announce: false,
            ..NetConfig::lan(0, "horse-jump-sound-test".into(), 3)
        },
        HubConfig {
            default_game_mode: 1,
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
    let (tx, rx) = mpsc::channel();
    let mut guest = Guest::new(
        Joined {
            conn: 2,
            uuid: 2,
            name: GUEST.into(),
            properties: Vec::new(),
            info: ClientInformation::new(2),
            brand: "test".into(),
            sender: PlayerSender::new(tx, Arc::default()),
            latency: Arc::new(AtomicI32::new(0)),
            address: "127.0.0.1:25565".parse().unwrap(),
            expected: false,
        },
        2,
        [4.5, 100.0, 6.5],
        1,
    );
    guest.placed = true;
    guest.loaded = true;
    (guest.vehicle, guest.vehicle_order) = (Some(horse), 1);
    game.lan.as_mut().unwrap().hub.guests.push(guest);
    (game, rx)
}

/// The keys the guest's last `ServerboundPlayerInput` said (`g.input`, what
/// the hub's handler records; 16 is jump), then the guest's server tick, then
/// the sounds it captured go out.
fn tick_guest(game: &mut Game<'_>, flags: u8) {
    game.lan
        .as_mut()
        .unwrap()
        .hub
        .guests
        .update_connection(2, |g| g.input = flags);
    game.tick_guests();
    game.lan_broadcast_effects();
}

/// The volume and pitch of each `entity.horse.jump` the guest's client was
/// told to play.
fn jump_sounds(game: &Game<'_>, rx: &mpsc::Receiver<Out>) -> Vec<(f32, f32)> {
    let host = game.lan.as_ref().unwrap();
    rx.try_iter()
        .flat_map(|out| match out {
            Out::Packet(body) => vec![body],
            Out::Packets(bodies) => bodies,
            Out::Disconnect(_) => Vec::new(),
        })
        .filter_map(|body| {
            let mut r = horizon_protocol::buf::Reader::new(&body);
            let id = r.varint().ok()?;
            match play::Clientbound::decode(id, r.rest()).ok()? {
                play::Clientbound::Sound(s)
                    if host.hub.registry_name("minecraft:sound_event", s.sound.0) == Some(JUMP) =>
                {
                    Some((s.volume, s.pitch))
                }
                _ => None,
            }
        })
        .collect()
}

/// Holding the jump bar and letting it go: the horse rears and plays its jump
/// sound once, from `handleStartJump`'s `playJumpSound`.
#[test]
#[ignore = "daily"]
fn a_guests_riding_jump_plays_the_jump_sound_once() {
    let (mut game, rx) = game();
    for _ in 0..10 {
        tick_guest(&mut game, 16);
    }
    // The client lets the bar go: its input clears, then it sends the jump.
    tick_guest(&mut game, 0);
    game.lan_guest_packet(
        2,
        S::PlayerCommand(play::PlayerCommand {
            id: 2,
            action: 3,
            data: 100,
        }),
    );
    assert_eq!(
        jump_sounds(&game, &rx),
        [(0.4, 1.0)],
        "one jump, one playJumpSound at 0.4 and 1.0"
    );
}
