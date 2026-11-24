#![cfg(test)]
use super::*;
use crate::game::{Audio, Options, Progress, Settings, load_resources};
use horizon_client::{
    chunk::Chunk,
    lan::{Joined, NetConfig, Out, PlayerSender, guest::Guest, hub::HubConfig},
};
use horizon_core::position::ChunkPos;
use horizon_protocol::packets::configuration::ClientInformation;
use std::sync::{Arc, Mutex, atomic::AtomicI32, mpsc};

/// A mod that claims a right click and records the entity, its type and the
/// point clicked: the listener #19010 says never hears `entity_interact`.
struct Probe {
    seen: Arc<Mutex<Vec<(i32, String, [f64; 3])>>>,
}

#[mod_api::__subscriptions]
impl mod_api::Hooks for Probe {
    fn entity_interact_specific(
        &self,
        _host: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::EntityInteractSpecificEvent<'_>,
    ) -> mod_api::Flow {
        self.seen
            .lock()
            .unwrap()
            .push((event.entity, event.kind.to_string(), event.location));
        mod_api::Flow::Cancel
    }
}

/// A server's game on the prepared resources, or None without them (CI carries
/// no vanilla data). After the data check a failure is a broken fixture, not a
/// skipped test.
fn server_game<'a>(
    resources: &'a mut Option<Arc<crate::game::Resources>>,
) -> Option<Box<Game<'a>>> {
    let root = std::env::var_os("HORIZON_TEST_RESOURCES")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../.local/horizon-client")
                .join(horizon_core::VANILLA_VERSION)
        });
    if !root.join("movement-properties.nbt").exists() {
        return None;
    }
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
    .expect("the prepared resources load");
    let mut options = Options::parse(std::iter::empty())
        .expect("the default options parse")
        .expect("the default options are there");
    options.resources = root.clone();
    options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
    let res = resources.insert(loaded.resources);
    let mut game = Box::new(
        Game::new(
            res,
            loaded.pack,
            loaded.gui.expect("the resources carry a GUI state"),
            options,
            settings,
            Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
                root.clone(),
            )),
            None,
        )
        .expect("a server game starts on the prepared resources"),
    );
    game.reset_entity_residency();
    game.native_lifecycle
        .begin_open(mod_api::Authority::IntegratedServer);
    game.creative = false;
    game.options.dedicated = Some(Vec::new());
    Some(game)
}

/// The chunk at the origin in place, frozen, and the level open to one guest.
fn level_at_origin(game: &mut Game<'_>) {
    game.build_registries();
    game.set_allow_commands(true);
    let stone = game
        .pack
        .assets
        .registry
        .default_state("minecraft:stone")
        .unwrap()
        .0;
    let mut chunk = Chunk::empty(0, 0);
    for x in 0..16 {
        for z in 0..16 {
            chunk.set(x, 99, z, stone);
        }
    }
    game.world.chunks.insert((0, 0), Arc::new(chunk));
    game.block_ticks
        .add_empty_chunk(ChunkPos::new(0, 0))
        .unwrap();
    assert!(game.run_command("forceload add 0 0"));
    assert!(game.run_command("tick freeze"));
}

/// The level's own `minecraft:interaction`, summoned from `snbt` with the width
/// and height `Interaction.makeBoundingBox` reads.
fn summon_interaction(game: &mut Game<'_>, snbt: &str) -> i32 {
    assert!(
        game.run_command(&format!("summon minecraft:interaction {snbt}")),
        "the interaction summons"
    );
    game.others
        .iter()
        .find(|o| o.kind == "minecraft:interaction")
        .expect("the summoned interaction")
        .id as i32
}

/// One guest of the level open, standing at `at`, and what its connection
/// writes back.
fn join_guest(game: &mut Game<'_>, at: [f64; 3]) -> mpsc::Receiver<Out> {
    game.lan_host(
        super::super::lan::LanSettings {
            port: 0,
            guest_commands: true,
            force_game_mode: false,
        },
        NetConfig {
            listen: false,
            announce: false,
            ..NetConfig::loopback(0, "interaction-box-unit".into(), 2)
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
    .expect("the level opens to a guest");
    let (tx, rx) = mpsc::channel::<Out>();
    let me = game.take_entity_id() as i32;
    let mut guest = Guest::new(
        Joined {
            conn: 1,
            uuid: 1,
            name: "Clicker".into(),
            properties: Vec::new(),
            info: ClientInformation::new(2),
            brand: "unit".into(),
            sender: PlayerSender::new(tx, Arc::default()),
            latency: Arc::new(AtomicI32::new(0)),
            address: "127.0.0.1:1".parse().unwrap(),
            expected: false,
        },
        me,
        at,
        2,
    );
    guest.placed = true;
    guest.loaded = true;
    guest.load_timeout = 0;
    guest.dimension = "minecraft:overworld".into();
    game.lan.as_mut().unwrap().hub.guests.push(guest);
    game.sync_tick_chunks();
    rx
}

/// The level's own `minecraft:interaction` is a guest's click target: the box
/// `ServerGamePacketListenerImpl.handleInteract` measures the 3-block range
/// from is `Interaction.makeBoundingBox` of its saved width and height, so a
/// right click reaches `Player.interactOn` instead of being refused as an
/// invalid target, and the mods' `entity_interact_specific` hears it. #19010
#[test]
fn a_guests_click_reaches_the_levels_own_interaction() {
    let mut resources = None;
    let Some(mut game) = server_game(&mut resources) else {
        return;
    };
    level_at_origin(&mut game);
    // A 2 × 3 interaction: `Interaction.makeBoundingBox` is `x ± width / 2`,
    // `y .. y + height`, `z ± width / 2` from its position, so its box is
    // [-0.5, 100, -0.5] .. [1.5, 103, 1.5].
    let interaction = summon_interaction(&mut game, "0.5 100 0.5 {width:2f,height:3f,response:1b}");
    // Three blocks off the box's near face, inside the interaction range.
    let _rx = join_guest(&mut game, [0.5, 100.0, 4.5]);

    let target = game.guest_target_box(0, interaction).expect("the box");
    let (lo, hi) = (
        target.min_position().components(),
        target.max_position().components(),
    );
    assert_eq!(lo, [-0.5, 100.0, -0.5], "makeBoundingBox min");
    assert_eq!(hi, [1.5, 103.0, 1.5], "makeBoundingBox max");

    // The packet's location is measured from the entity's position; a point on
    // the box's near face, so the click is on the interaction.
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![mod_api::ModEntry::new(
        "probe",
        Box::new(Probe { seen: seen.clone() }),
    )];
    let used = mod_platform::testing::with_mods(mods.into(), || {
        game.guest_interact(1, 0, interaction, Hand::Main, [0.25, 1.5, 1.0])
    });
    assert_eq!(
        used,
        Ok(true),
        "a claim is `Player.interactOn` taking the click: {used:?}"
    );
    let seen = seen.lock().unwrap();
    assert_eq!(
        *seen,
        vec![(
            interaction,
            "minecraft:interaction".to_owned(),
            [0.25, 1.5, 1.0]
        )],
        "the hook hears the clicked entity, its type and the point clicked"
    );
}

/// The box now lets a guest's click reach `Player.attack`, which picks its
/// target again: it must not find the mob behind the interaction.
/// `Interaction.skipAttackInteraction` keeps the blow on the interaction, so a
/// left click on it hurts nobody (#19010).
#[test]
fn a_guests_left_click_does_not_reach_the_mob_behind_the_interaction() {
    let mut resources = None;
    let Some(mut game) = server_game(&mut resources) else {
        return;
    };
    level_at_origin(&mut game);
    // The box is [0, 100, 2.5] .. [1, 103, 3.5], its middle [0.5, 101.5, 3.0].
    let interaction = summon_interaction(&mut game, "0.5 100 3.0 {width:1f,height:3f,response:1b}");
    // A pig behind the box, in the ray the server aims at the box's middle
    // (the eye is at 101.62, the ray drops 0.12 over the 1.5 blocks): a strike
    // that picks its target again hits it.
    assert!(
        game.run_command("summon pig 0.5 101 1.9 {NoAI:1b,NoGravity:1b}"),
        "the pig summons"
    );
    let pig = game
        .mobs
        .iter()
        .find(|m| m.kind.id == "minecraft:pig")
        .expect("the summoned pig")
        .id as i32;
    let _rx = join_guest(&mut game, [0.5, 100.0, 4.5]);

    let health = |game: &Game<'_>| {
        game.mobs
            .iter()
            .find(|m| m.id as i32 == pig)
            .expect("the pig")
            .health
    };
    let full = health(&game);
    game.lan_guest_packet(1, S::Attack(play::Attack { id: interaction }));
    assert_eq!(
        health(&game),
        full,
        "a left click on the interaction hurts nobody"
    );

    // The contrast: the same guest's click on the pig itself does hurt it.
    game.lan_guest_packet(1, S::Attack(play::Attack { id: pig }));
    assert!(
        health(&game) < full,
        "the click on the pig itself lands: {} of {full} left",
        health(&game)
    );
}
