//! `ShelfRenderer.extractRenderState` hands a shelf's items the `ShelfBlockEntity`
//! as their `ItemOwner` — the only block entity renderer that passes one: its
//! `position()` is the block's centre and its `getVisualRotationYInDegrees()` is
//! `facing.getOpposite().toYRot()`. A compass on a shelf therefore points from the
//! shelf's own block: two shelves at different places and facings draw different
//! needles, where with no owner both drew the same one.
use super::*;
use crate::game::{
    Audio, Options, Progress, Settings, execution_role::ExecutionRole, load_resources,
};
use horizon_client::{
    chunk::Chunk,
    containers::{BlockEntity, Shelf},
    items::ItemStack,
};
use horizon_core::nbt::{Compound, Tag};

/// The lodestone every compass here tracks, in this dimension.
const TARGET: [i32; 3] = [3, 200, 1];

/// A compass with `minecraft:lodestone_tracker` pointing at [`TARGET`].
fn lodestone_compass() -> ItemStack {
    let mut stack = ItemStack::new("minecraft:compass", 1);
    stack.data_mut().set_other(
        "minecraft:lodestone_tracker",
        Some(Tag::Compound(Compound::from([
            (
                "target".into(),
                Tag::Compound(Compound::from([
                    (
                        "dimension".into(),
                        Tag::String("minecraft:overworld".into()),
                    ),
                    ("pos".into(), Tag::IntArray(TARGET.to_vec())),
                ])),
            ),
            ("tracked".into(), Tag::Byte(1)),
        ]))),
    );
    stack
}

/// A client game that draws: its item models (the compass' `range_dispatch`) and
/// atlas come from the resources, as `ShelfRenderer` gets them.
fn drawing_game() -> Game<'static> {
    let root = std::path::PathBuf::from(
        std::env::var_os("HORIZON_TEST_RESOURCES").expect("HORIZON_TEST_RESOURCES"),
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
        crate::game::execution_role::ExecutionRole::Client,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = ExecutionRole::Client;
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
    game.world
        .chunks
        .insert((0, 0), std::sync::Arc::new(Chunk::empty(0, 0)));
    game
}

/// `minecraft:oak_shelf` with `facing` at `pos`, its block entity holding `stack`
/// in the middle slot.
fn place_shelf(game: &mut Game<'_>, pos: [i32; 3], facing: &str, stack: &ItemStack) {
    let state = {
        let registry = &game.pack.assets.registry;
        let mut state = registry
            .state(registry.default_state("minecraft:oak_shelf").unwrap())
            .unwrap()
            .clone();
        state.properties.insert("facing".into(), facing.into());
        registry.resolve(&state).unwrap()
    };
    game.world.set(pos[0], pos[1], pos[2], state).unwrap();
    let mut shelf = Shelf::default();
    shelf.items[1] = Some(stack.clone());
    game.world.block_entities.insert(
        (pos[0], pos[1], pos[2]),
        BlockEntity::Shelf(Box::new(shelf)),
    );
}

/// What the shelf at `pos` draws this frame.
fn drawn_vertices(game: &Game<'_>, pos: [i32; 3]) -> Vec<Vertex> {
    let camera = glam::DVec3::new(
        f64::from(pos[0]) + 0.5,
        f64::from(pos[1]) + 0.5,
        f64::from(pos[2]) + 0.5,
    );
    let mut vertices = Vec::new();
    game.shelf_geometry(camera, &mut vertices);
    vertices
}

/// The sprite every one of `vertices` samples, so the model the compass'
/// `range_dispatch` chose: `item/compass_16`..`item/compass_31`, one per needle
/// angle. `None` when nothing was drawn.
fn drawn_sprite(game: &Game<'_>, vertices: &[Vertex]) -> Option<String> {
    if vertices.is_empty() {
        return None;
    }
    game.gui
        .atlas
        .sprites
        .iter()
        .find(|(_, rect)| {
            vertices.iter().all(|v| {
                (rect.u0..=rect.u1).contains(&v.uv[0]) && (rect.v0..=rect.v1).contains(&v.uv[1])
            })
        })
        .map(|(name, _)| name.clone())
}

#[test]
#[ignore = "daily"]
fn a_compass_on_a_shelf_points_from_that_shelf() {
    let mut game = drawing_game();
    let compass = lodestone_compass();
    let south = [0, 200, 0];
    place_shelf(&mut game, south, "south", &compass);
    let south_sprite = drawn_sprite(&game, &drawn_vertices(&game, south));
    game.world
        .block_entities
        .remove(&(south[0], south[1], south[2]));
    let north = [0, 200, 4];
    place_shelf(&mut game, north, "north", &compass);
    let north_sprite = drawn_sprite(&game, &drawn_vertices(&game, north));
    // `CompassAngleState.calculate`: the needle is `0.5 - (ownerY - 0.25 - angle)`,
    // `ownerY` the owner's `getVisualRotationYInDegrees() / 360` and `angle` the
    // `atan2` to the target's centre over `2 * PI`; the compass' `range_dispatch`
    // (`scale: 32`, thresholds 0.0 then 0.5, 1.5, ...) names the frame. A south
    // shelf at (0, 200, 0) turns its owner 180 (its facing's opposite) and has the
    // target 3 east and 1 south: 0.301 * 32 = 9.6, so `compass_26`. A north shelf at
    // (0, 200, 4) turns it 0 and has the target 3 east and 3 north: 0.625 * 32 =
    // 20.0, so `compass_04`.
    assert_eq!(
        south_sprite,
        Some("item/compass_26".to_owned()),
        "the south shelf's needle (no sprite: it drew no compass)"
    );
    assert_eq!(
        north_sprite,
        Some("item/compass_04".to_owned()),
        "the north shelf's needle (no sprite: it drew no compass)"
    );
}
