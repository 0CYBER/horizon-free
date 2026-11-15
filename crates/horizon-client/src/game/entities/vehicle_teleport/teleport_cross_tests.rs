//! `/tp` of hanging entities (`BlockAttachedEntity.setPos`) and of any entity
//! kind into another dimension (`Entity.teleportCrossDimension`).
use crate::game::{Audio, Dim, Game, Options, Progress, Settings, execution_role, load_resources};
use horizon_client::chunk::Chunk;
use horizon_core::position::ChunkPos;
use std::sync::Arc;

fn game() -> Game<'static> {
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
    options.execution_role = execution_role::ExecutionRole::IntegratedServer;
    let res: &'static _ = Box::leak(Box::new(loaded.resources));
    let mut game = Game::new(
        res,
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
    game.reset_entity_residency();
    game.native_lifecycle
        .begin_open(mod_api::Authority::IntegratedServer);
    game.build_registries();
    game.set_allow_commands(true);
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    game.block_ticks
        .add_empty_chunk(ChunkPos::new(0, 0))
        .unwrap();
    game
}

/// `BlockAttachedEntity.setPos`: the block it hangs in is the floor of the
/// requested position; its bounds follow from it.
#[test]
#[ignore = "daily"]
fn tp_moves_a_frame_and_a_painting() {
    let mut game = game();
    assert!(game.run_command("summon minecraft:painting 0 2 0 {facing:0b}"));
    assert!(game.run_command("summon minecraft:item_frame 0 2 2 {Facing:3b}"));
    assert!(game.run_command("tp @e[type=minecraft:painting,limit=1] 3 2 0"));
    assert!(game.run_command("tp @e[type=minecraft:item_frame,limit=1] 3 2 2"));
    assert_eq!(game.paintings.len(), 1);
    assert_eq!(game.paintings[0].pos, [3, 2, 0]);
    assert_eq!(game.item_frames.len(), 1);
    assert_eq!(game.item_frames[0].pos, [3, 2, 2]);
}

/// `Entity.teleportTo(ServerLevel, ..)` -> `teleportCrossDimension` for a
/// target in another level than the command source's.
#[test]
#[ignore = "daily"]
fn every_entity_kind_follows_a_cross_dimension_tp() {
    let mut game = game();
    let back = game.enter_command_level(Dim::Nether);
    assert_eq!(game.dim, Dim::Nether);
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    let _ = game.block_ticks.add_empty_chunk(ChunkPos::new(0, 0));
    if let Some(back) = back {
        game.enter_command_level(back);
    }
    for command in [
        "summon minecraft:armor_stand 0.5 2 0.5",
        "summon minecraft:item 1.5 2 0.5 {Item:{id:\"minecraft:stone\",count:1}}",
        "summon minecraft:item_frame 2.5 2 0.5 {Facing:3b}",
        "summon minecraft:arrow 3.5 2 0.5 {Motion:[0.0d,0.0d,0.0d]}",
        "summon minecraft:tnt 4.5 2 0.5",
    ] {
        assert!(game.run_command(command), "{command}");
    }
    for (ty, x) in [
        ("minecraft:armor_stand", 10.5),
        ("minecraft:item", 11.5),
        ("minecraft:item_frame", 12.5),
        ("minecraft:arrow", 13.5),
        ("minecraft:tnt", 14.5),
    ] {
        let command =
            format!("execute in minecraft:the_nether run tp @e[type={ty},limit=1] {x} 64 0.5");
        assert!(game.run_command(&command), "{command}");
    }
    assert!(game.armor_stands.is_empty(), "the stand left the overworld");
    assert!(game.dropped.is_empty(), "the item left the overworld");
    assert!(game.item_frames.is_empty(), "the frame left the overworld");
    assert!(
        game.creatures.projectiles.is_empty(),
        "the arrow left the overworld"
    );
    assert!(game.tnts.is_empty(), "the tnt left the overworld");
    let back = game.enter_command_level(Dim::Nether);
    assert_eq!(game.dim, Dim::Nether);
    let stand = game.armor_stands.iter().next().expect("stand crossed");
    assert_eq!(
        [stand.position.x, stand.position.y, stand.position.z],
        [10.5, 64.0, 0.5]
    );
    let item = game.dropped.iter().next().expect("item crossed");
    assert_eq!(
        [item.position.x, item.position.y, item.position.z],
        [11.5, 64.0, 0.5]
    );
    let frame = game.item_frames.first().expect("frame crossed");
    assert_eq!(frame.pos, [12, 64, 0]);
    let arrow = game
        .creatures
        .projectiles
        .iter()
        .next()
        .expect("arrow crossed");
    assert_eq!(
        [arrow.position.x, arrow.position.y, arrow.position.z],
        [13.5, 64.0, 0.5]
    );
    let tnt = game.tnts.first().expect("tnt crossed");
    assert_eq!(
        [tnt.position.x, tnt.position.y, tnt.position.z],
        [14.5, 64.0, 0.5]
    );
    // `PrimedTnt.teleport`: the new entity has used a portal.
    assert!(tnt.used_portal, "the arrived tnt ignores its portal");
    if let Some(back) = back {
        game.enter_command_level(back);
    }
}

/// `Level.isInSpawnableBounds` on `BlockPos.containing`: the lower bounds are
/// inside, the upper bounds outside, so `/tp` accepts the world border
/// coordinates. Pins the bound (already true before the cross-dimension move).
#[test]
#[ignore = "daily"]
fn tp_accepts_the_spawnable_border_coordinates() {
    let mut game = game();
    assert!(game.run_command("summon minecraft:armor_stand 0.5 2 0.5"));
    for command in [
        "tp @e[type=minecraft:armor_stand,limit=1] -30000000 64 0",
        "tp @e[type=minecraft:armor_stand,limit=1] 0 -20000000 0",
    ] {
        assert!(game.run_command(command), "{command}");
    }
    for command in [
        "tp @e[type=minecraft:armor_stand,limit=1] -30000001 64 0",
        "tp @e[type=minecraft:armor_stand,limit=1] 0 -20000001 0",
        "tp @e[type=minecraft:armor_stand,limit=1] 30000000 64 0",
    ] {
        assert!(!game.run_command(command), "{command}");
    }
}
