//! The random behind a thrown item and behind an item's sound: the thrower's
//! own (`Entity.random`, `LivingEntity.createItemStackToDrop`) or a client's
//! (`ClientPacketListener.random`), never the level's, whose draws are what
//! random ticks, leaf decay and spawns come from.
use super::*;
use crate::game::game_tests::tests::server_game;
use crate::game::{Audio, Options, Progress, Settings, load_resources};
use horizon_client::chunk::Chunk;
use horizon_client::entities::{ExperienceOrb, ItemEntity};
use horizon_client::items::ItemStack;
use horizon_protocol::packets::play::{self, Clientbound as C};
use std::sync::Arc;

/// A game with the player standing at 8.5 70 8.5 in a ticking chunk, with the
/// positional sounds it plays recorded.
fn standing_game() -> Game<'static> {
    let resources = Box::leak(Box::new(None));
    let mut game = server_game(resources);
    let feet = Vec3::new(8.5, 70.0, 8.5);
    game.player = Some(horizon_client::motion::PlayerMotion::new(game.motion, feet).unwrap());
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    game.load_ticking_neighbourhood(0, 0);
    game.update_chunk_tickets();
    assert!(game.entity_ticking_area()(8, 8));
    game.ticked_entities_through = u32::MAX;
    game.audio.capture = Some(Vec::new());
    game
}

/// The one sound `event` played since the last call, with its volume.
fn sound(game: &mut Game<'_>, event: &str) -> (f32, f32) {
    let heard = game.audio.capture.replace(Vec::new()).unwrap_or_default();
    let sound = heard
        .iter()
        .find(|s| s.event == event)
        .unwrap_or_else(|| panic!("no {event} sound among {heard:?}"));
    (sound.volume, sound.pitch)
}

/// `Player.createItemStackToDrop(false)`: the drop key's throw is 0.3 along
/// the view (level facing, so +z) and 0.1 up, scattered by the thrower's own
/// random, which the level's draws never see.
#[test]
#[ignore = "daily"]
fn a_thrown_item_takes_the_throwers_random_not_the_levels() {
    let mut game = standing_game();
    game.yaw = 0.0;
    game.pitch = 0.0;
    game.inventory[game.selected] = Some(ItemStack::new("minecraft:stone", 4));
    let level = game.world.random.state();
    game.drop_held(false);
    let thrown = game.dropped.last().expect("the drop key threw the stack");
    let velocity = thrown.velocity;
    assert!(
        (velocity.z - 0.3).abs() <= 0.02,
        "the eye throw goes 0.3 along the view: {velocity:?}"
    );
    assert!(velocity.x.abs() <= 0.02, "{velocity:?}");
    assert!((0.0..=0.2).contains(&velocity.y), "{velocity:?}");
    assert_eq!(
        game.world.random.state(),
        level,
        "the throw drew from the level's random"
    );
    // The next throw scatters differently: the thrower's stream is live.
    game.drop_held(false);
    assert_ne!(game.dropped.last().unwrap().velocity, velocity);
    assert_eq!(game.world.random.state(), level);
}

/// `Player.dropEquipment` -> `Inventory.dropAll(randomly)`: each stack a dying
/// player lets go rises 0.2, scattered by the thrower's own random.
#[test]
#[ignore = "daily"]
fn a_death_scatter_takes_the_throwers_random_not_the_levels() {
    let mut game = standing_game();
    game.inventory[0] = Some(ItemStack::new("minecraft:stone", 2));
    let level = game.world.random.state();
    game.drop_inventory(
        [8.5, 70.0, 8.5],
        &crate::game::creatures::DyingDrops {
            entity: 0,
            kind: "minecraft:player",
            player: None,
            source: None,
            recently_hit: false,
        },
    );
    let thrown = game.dropped.first().expect("the stack was thrown");
    assert_eq!(thrown.velocity.y, f64::from(0.2f32), "a scatter rises 0.2");
    assert!(thrown.velocity.x.abs() <= 0.5 && thrown.velocity.z.abs() <= 0.5);
    assert_eq!(
        game.world.random.state(),
        level,
        "the scatter drew from the level's random"
    );
}

/// `ClientPacketListener.handleTakeItemEntity` pitches the pickup from the
/// client's own random; `GiveCommand.giveItem` uses the player's. Neither is
/// the level's, which random ticks draw from.
#[test]
#[ignore = "daily"]
fn a_pickup_pitch_is_not_drawn_from_the_levels_random() {
    let mut game = standing_game();
    game.spawn_item(
        ItemStack::new("minecraft:stone", 2),
        [8.5, 70.0, 8.5],
        [0.0; 3],
        0,
    );
    let level = game.world.random.state();
    game.tick_items();
    assert!(game.dropped.is_empty(), "the item was picked up");
    let (volume, pitch) = sound(&mut game, "minecraft:entity.item.pickup");
    assert_eq!(volume, 0.2);
    // `(nextFloat - nextFloat) * 1.4 + 2`.
    assert!((0.6..=3.4).contains(&pitch), "pitch {pitch}");
    assert_eq!(
        game.world.random.state(),
        level,
        "the pickup drew from the level's random"
    );

    game.build_registries();
    game.set_allow_commands(true);
    let level = game.world.random.state();
    assert!(game.run_command("give @s minecraft:stone 2"));
    let (volume, pitch) = sound(&mut game, "minecraft:entity.item.pickup");
    assert_eq!(volume, 0.2);
    assert!((0.6..=3.4).contains(&pitch), "pitch {pitch}");
    assert_eq!(
        game.world.random.state(),
        level,
        "/give drew from the level's random"
    );
}

/// A window joined to a server through memory, with the sounds it plays
/// recorded: the client half of a pickup (`ClientPacketListener`).
fn with_remote_game<R>(f: impl FnOnce(&mut Game<'_>) -> R) -> R {
    let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
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
    let mut game = Game::new(
        &loaded.resources,
        loaded.pack,
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
            root.to_path_buf(),
        )),
        None,
    )
    .unwrap();
    let (pipe, _server) = horizon_protocol::connection::Pipe::pair();
    game.connect_local(pipe, "Steve").unwrap();
    game.audio.capture = Some(Vec::new());
    f(&mut game)
}

/// `ClientPacketListener.handleTakeItemEntity`: a client pitches an item's and
/// an orb's pickup from its own random (`ClientPacketListener.random`), so a
/// pickup never draws the level's stream, which its particles and ambient
/// sounds share.
#[test]
#[ignore = "daily"]
fn a_client_pickup_sound_takes_the_listeners_random() {
    with_remote_game(|game| {
        let at = Vec3::new(2.5, 64.0, 2.5);
        game.dropped.push(ItemEntity::new(
            9,
            ItemStack::new("minecraft:stone", 1),
            at,
            Vec3::ZERO,
            0.0,
        ));
        game.orbs.push(ExperienceOrb::new(10, at, Vec3::ZERO, 1));
        let take = |item: i32| {
            C::TakeItemEntity(play::TakeItemEntity {
                item,
                player: 1,
                amount: 1,
            })
        };
        let level = game.world.random.state();
        game.handle_packet(take(9)).unwrap();
        let (volume, pitch) = sound(game, "minecraft:entity.item.pickup");
        assert_eq!(volume, 0.2);
        assert!((0.6..=3.4).contains(&pitch), "pitch {pitch}");
        assert_eq!(
            game.world.random.state(),
            level,
            "the client's pickup drew from the level's random"
        );
        game.handle_packet(take(10)).unwrap();
        let (volume, pitch) = sound(game, "minecraft:entity.experience_orb.pickup");
        assert_eq!(volume, 0.1);
        assert!((0.55..1.25).contains(&pitch), "pitch {pitch}");
        assert_eq!(
            game.world.random.state(),
            level,
            "the client's orb pickup drew from the level's random"
        );
    });
}

/// `Entity.lavaHurt` pitches the burn sound from the item's own random, the
/// one its constructor drew from.
#[test]
#[ignore = "daily"]
fn a_burning_items_sound_is_not_drawn_from_the_levels_random() {
    let mut game = standing_game();
    let lava = game
        .pack
        .assets
        .registry
        .default_state("minecraft:lava")
        .unwrap();
    game.world.set(12, 70, 8, lava).unwrap();
    game.spawn_item(
        ItemStack::new("minecraft:stone", 1),
        [12.5, 70.0, 8.5],
        [0.0; 3],
        0,
    );
    // `Entity.lavaHurt` sounds on the tenth tick of a contact that leaves the
    // item alive (`should_playLavaHurtSound`), so the item is about to be 10.
    game.dropped[0].ticks = 9;
    let level = game.world.random.state();
    game.tick_items();
    let (volume, pitch) = sound(&mut game, "minecraft:entity.generic.burn");
    assert_eq!(volume, 0.4);
    // `2.0 + nextFloat * 0.4`.
    assert!((2.0..2.4).contains(&pitch), "pitch {pitch}");
    assert_eq!(
        game.world.random.state(),
        level,
        "the burn sound drew from the level's random"
    );
}
