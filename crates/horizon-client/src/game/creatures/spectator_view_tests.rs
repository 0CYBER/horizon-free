//! `Entity.isInvisibleTo`: a spectator sees an invisible body as the translucent
//! one (`LivingEntityRenderer.submit`'s `forceTransparent`), anyone else not at all.
use crate::game::{Audio, Game, Options, Progress, Settings, execution_role, load_resources};
use horizon_client::chunk::Chunk;
use horizon_client::command::args::GameMode;
use horizon_client::mobs::{Mob, MobTypes};
use horizon_core::geometry::Vec3;
use horizon_core::position::ChunkPos;
use std::sync::Arc;

/// The prepared vanilla resources.
fn root() -> std::path::PathBuf {
    std::env::var_os("HORIZON_TEST_RESOURCES")
        .map(std::path::PathBuf::from)
        .expect("set HORIZON_TEST_RESOURCES to prepared resources")
}

fn game() -> Game<'static> {
    let root = root();
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
    game.build_registries();
    // The atlas the game bakes at startup: `mob_geometry` finds a body's texture in it.
    game.gui.atlas = horizon_client::hud::HudAtlas::build(
        &horizon_assets::resource_source::ResourceSource::directory(root.clone()),
    )
    .unwrap();
    game.set_allow_commands(true);
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    game.block_ticks
        .add_empty_chunk(ChunkPos::new(0, 0))
        .unwrap();
    for command in ["forceload add 0 0", "fill 0 63 0 15 63 15 stone"] {
        assert!(game.run_command(command), "{command}");
    }
    game
}

/// Like [`game`], but loads resources with `ExecutionRole::Client` so block models
/// (`model_catalog` and `layer_names`) are baked for `block_model_vertices` /
/// `sulfur_cube_body_geometry`; `UiResources::load` builds `gui.atlas`.
fn client_game() -> Game<'static> {
    let root = root();
    let settings = Settings::default();
    let loaded = load_resources(
        horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        crate::game::execution_role::ExecutionRole::Client,
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
    game.build_registries();
    game.set_allow_commands(true);
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    game.block_ticks
        .add_empty_chunk(ChunkPos::new(0, 0))
        .unwrap();
    for command in ["forceload add 0 0", "fill 0 63 0 15 63 15 stone"] {
        assert!(game.run_command(command), "{command}");
    }
    game
}

/// `/effect give @e[type=minecraft:pig,limit=1] minecraft:invisibility 100 0`
/// sets the shared invisible flag a client reads (`LivingEntity.updateInvisibilityStatus`).
/// `Entity.isInvisibleTo` is false for a spectator, so `LivingEntityRenderer.submit`
/// draws the body it cannot show as visible as `entityTranslucentCull` at
/// `ARGB.multiply(654311423, getModelTint)`: alpha 38, in the blending pass.
#[test]
#[ignore = "daily"]
fn a_spectator_sees_the_invisible_body() {
    let mut game = game();
    let types = MobTypes::load(&root()).unwrap();
    let kind = Arc::new(types.types["minecraft:pig"].clone());
    let mut pig = Mob::new(1, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
    pig.invisible = true;
    game.mobs.push(pig);
    let camera = glam::DVec3::new(6.5, 65.0, 6.5);

    // Anyone but a spectator sees no body at all.
    let mut out = Vec::new();
    game.mob_geometry(camera, 0.0, &mut out);
    assert!(out.is_empty(), "an invisible body is hidden from a player");

    // A spectator sees it forced translucent.
    game.set_mode(GameMode::Spectator);
    let mut out = Vec::new();
    game.mob_geometry(camera, 0.0, &mut out);
    assert!(!out.is_empty(), "a spectator sees the invisible body");
    assert!(
        out.iter().all(|v| v.color >> 24 == 0x26),
        "the body is forced translucent (alpha 38)"
    );
    assert!(
        out.iter()
            .all(|v| v.layer & horizon_client::gfx::OIT_ENTITY != 0),
        "the translucent body blends"
    );
}

/// `SulfurCubeInnerLayer.submit` draws a contained block without checking
/// `state.isInvisible`, while the fallback inner core checks `!state.isInvisible`.
#[test]
#[ignore = "daily"]
fn an_invisible_sulfur_cube_keeps_its_contained_block() {
    let mut game = client_game();
    let types = MobTypes::load(&root()).unwrap();
    let kind = Arc::new(types.types["minecraft:sulfur_cube"].clone());
    let mut cube = Mob::new(1, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
    cube.invisible = true;
    cube.extra.set_body(Some("minecraft:tnt".into()));
    game.mobs.push(cube);
    let camera = glam::DVec3::new(6.5, 65.0, 6.5);

    // 1. Normal viewer + invisible cube with contained block: only the block draws.
    let mut out = Vec::new();
    game.mob_geometry(camera, 0.0, &mut out);
    assert!(
        !out.is_empty(),
        "an invisible sulfur cube keeps its contained block visible"
    );
    assert!(
        out.iter()
            .all(|v| v.layer & horizon_client::gfx::OIT_ENTITY == 0 && v.color >> 24 == 0xFF),
        "outer shell stays hidden; only opaque contained-block vertices emit"
    );

    // 2. Normal viewer + empty invisible cube: completely hidden.
    game.mobs[0].extra.set_body(None);
    let mut out = Vec::new();
    game.mob_geometry(camera, 0.0, &mut out);
    assert!(
        out.is_empty(),
        "an invisible empty sulfur cube is hidden from a normal viewer"
    );

    // 3. Spectator viewer: empty invisible cube shows only translucent outer shell (no inner core),
    //    while a cube with a contained block shows both the opaque block and translucent shell.
    game.set_mode(GameMode::Spectator);
    let mut out = Vec::new();
    game.mob_geometry(camera, 0.0, &mut out);
    assert!(
        !out.is_empty(),
        "a spectator sees the empty invisible cube's translucent outer shell"
    );
    assert!(
        out.iter()
            .all(|v| v.layer & horizon_client::gfx::OIT_ENTITY != 0 && v.color >> 24 == 0x26),
        "empty invisible cube omits the inner core in spectator view"
    );

    game.mobs[0].extra.set_body(Some("minecraft:tnt".into()));
    let mut out = Vec::new();
    game.mob_geometry(camera, 0.0, &mut out);
    assert!(
        out.iter()
            .any(|v| v.layer & horizon_client::gfx::OIT_ENTITY == 0 && v.color >> 24 == 0xFF),
        "spectator sees the opaque contained block"
    );
    assert!(
        out.iter()
            .any(|v| v.layer & horizon_client::gfx::OIT_ENTITY != 0 && v.color >> 24 == 0x26),
        "spectator also sees the translucent outer shell"
    );

    // 4. Normal viewer + glowing + invisible cube with contained block:
    //    contained block stays visible and receives the glowing outline.
    game.set_mode(GameMode::Survival);
    game.mobs[0].glowing = true;
    game.glowing_vertices.borrow_mut().clear();
    let mut out = Vec::new();
    game.mob_geometry(camera, 0.0, &mut out);
    assert!(
        !out.is_empty(),
        "a glowing invisible sulfur cube keeps its contained block visible"
    );
    assert!(
        out.iter()
            .all(|v| v.layer & horizon_client::gfx::OIT_ENTITY == 0 && v.color >> 24 == 0xFF),
        "only the contained block remains visible for a glowing invisible cube"
    );
    assert!(
        !game.glowing_vertices.borrow().is_empty(),
        "the contained block receives the glowing outline"
    );
}
