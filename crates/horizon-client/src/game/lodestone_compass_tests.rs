//! Lodestone compasses: `ItemFrameRenderer.extractRenderState` hands the frame
//! entity to the item model, so a framed compass's needle reads the frame's
//! position and yaw; and only `CompassItem.inventoryTick` ticks a
//! `LodestoneTracker`, so any other item carrying the component keeps it.
use crate::game::native_machines::tests::server_game;
use crate::game::{Audio, Game, Options, Progress, Settings, execution_role, load_resources};
use horizon_client::{entities::ItemFrame, items::ItemStack};
use horizon_core::random::{EntityRandom, LegacyRandom};
use horizon_world::nbt::Tag;

/// The lodestone at `pos` (the `target` of a `LodestoneTracker`).
fn tracker(pos: [i32; 3]) -> Tag {
    Tag::compound([(
        "target",
        Tag::compound([
            ("dimension", Tag::String("minecraft:overworld".into())),
            ("pos", Tag::IntArray(pos.to_vec())),
        ]),
    )])
}

/// A frame holding a lodestone compass aimed at `5 70 16`.
fn framed_compass(id: u32, pos: [i32; 3], facing: u8, rotation: i8) -> ItemFrame {
    let mut compass = ItemStack::new("minecraft:compass", 1);
    compass
        .data_mut()
        .set_other("minecraft:lodestone_tracker", Some(tracker([5, 70, 16])));
    ItemFrame {
        id,
        pos,
        facing,
        glow: false,
        item: Some(compass),
        rotation,
        drop_chance: 1.0,
        fixed: false,
        invisible: false,
        check_interval: 0,
        custom_name: None,
        random: EntityRandom::own(LegacyRandom::new(0)),
    }
}

/// A game with the client's item definitions and atlas: resolving a stack to
/// its model needs both, and `server_game` loads neither.
fn client_game() -> Game<'static> {
    let root = std::path::PathBuf::from(
        std::env::var_os("HORIZON_TEST_RESOURCES")
            .expect("set HORIZON_TEST_RESOURCES to prepared resources"),
    );
    let settings = Settings::default();
    let loaded = load_resources(
        root.clone(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        false,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = execution_role::ExecutionRole::IntegratedServer;
    let res: &'static _ = Box::leak(Box::new(loaded.resources));
    Game::new(
        res,
        loaded.pack,
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&root),
        None,
    )
    .unwrap()
}

/// The `item/compass_NN` sprite the renderer draws for the frame's item: the
/// item quad's four atlas corners fall inside exactly one of the 32 compass
/// sprites `compass.json` dispatches over.
fn drawn_needle(game: &mut Game<'_>, frame: ItemFrame) -> String {
    game.item_frames = vec![frame];
    // The camera sits at the frame, so the frame passes `shouldRender`.
    let camera = glam::DVec3::from(super::center(&game.item_frames[0]));
    let mut out = Vec::new();
    game.frame_geometry(camera, &mut out);
    let near =
        |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).abs() < 1.0e-5 && (a[1] - b[1]).abs() < 1.0e-5;
    let drawn: Vec<&str> = game
        .gui
        .atlas
        .sprites
        .iter()
        .filter(|(name, rect)| {
            name.starts_with("item/compass_")
                && [
                    [rect.u0, rect.v0],
                    [rect.u0, rect.v1],
                    [rect.u1, rect.v0],
                    [rect.u1, rect.v1],
                ]
                .iter()
                .all(|corner| out.iter().any(|v| near(v.uv, *corner)))
        })
        .map(|(name, _)| name.as_str())
        .collect();
    assert_eq!(drawn.len(), 1, "the frame's item drew {drawn:?}");
    drawn[0].to_owned()
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_framed_lodestone_compass_needle_turns_with_its_frame() {
    let mut game = client_game();
    // `ItemFrame.position` is the block's centre, 0.46875 back along the
    // facing; `getVisualRotationYInDegrees` is 180 + the direction's 2D data
    // value * 90 + the item's rotation * 45. The lodestone at 5 70 16 is
    // south of the frames on the wall at z = 0.
    //
    // Facing south, `CompassAngleState` turns the yaw fraction (0.5) and the
    // angle to the lodestone's centre (0.25) into 0.5 - (0.5 - 0.25 - 0.25) =
    // 0.5, and the 32-step dispatch at 0.5 * 32 = 16 picks compass_00.
    let south = framed_compass(1, [5, 64, 1], 3, 0);
    assert_eq!(drawn_needle(&mut game, south), "item/compass_00");
    // Facing north instead: the yaw fraction drops to 0, the rotation to 0.
    let north = framed_compass(2, [5, 64, 1], 2, 0);
    assert_eq!(drawn_needle(&mut game, north), "item/compass_16");
    // A frame east of the lodestone: the angle opens up to 0.5305, so
    // 0.5305 * 32 = 16.98 clears the 16.5 threshold and picks compass_01.
    let east = framed_compass(3, [8, 64, 1], 3, 0);
    assert_eq!(drawn_needle(&mut game, east), "item/compass_01");
    // The frame's own item rotation turns it too: one 45 degree step takes the
    // yaw to -135, its fraction to 0.625, the rotation to 0.375, and
    // 0.375 * 32 = 12 clears the 11.5 threshold to pick compass_28.
    let rotated = framed_compass(4, [5, 64, 1], 3, 1);
    assert_eq!(drawn_needle(&mut game, rotated), "item/compass_28");
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn only_a_compass_ticks_its_lodestone_tracker() {
    let mut game = server_game();
    // The resident chunk (0, 0) holds no lodestone at 5 70 5: the target is gone.
    for (slot, item) in [(0, "minecraft:stick"), (1, "minecraft:compass")] {
        let mut stack = ItemStack::new(item, 1);
        stack
            .data_mut()
            .set_other("minecraft:lodestone_tracker", Some(tracker([5, 70, 5])));
        *game.stack_in_mut(slot) = Some(stack);
    }
    game.tick_lodestone_compasses();
    let target = |slot: usize| {
        game.stack_in(slot)
            .and_then(|stack| stack.data())
            .and_then(|data| data.other("minecraft:lodestone_tracker"))
            .and_then(|tracker| tracker.get("target"))
            .and_then(|target| target.get("pos").cloned())
    };
    // `Item.inventoryTick` is empty: a stick keeps what it carries.
    assert_eq!(target(0), Some(Tag::IntArray(vec![5, 70, 5])));
    // `CompassItem.inventoryTick` -> `LodestoneTracker.tick` clears it.
    assert_eq!(target(1), None);
}
