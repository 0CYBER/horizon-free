//! `ScaffoldingBlockItem.updatePlacementContext` reads the click's inside flag:
//! sneaking while the ray starts inside a scaffolding block places on the
//! opposite face, which the client's own raycast (`VoxelShape.clip`) supplies
//! and `BlockHitResult.isInside` carries.
use crate::game::{Game, Hit, game_tests::tests::server_game};
use horizon_client::{command::args::GameMode, items::ItemStack, motion::PlayerMotion};
use horizon_core::geometry::Vec3;
use winit::keyboard::KeyCode;

/// The block name at `pos`.
fn name(game: &Game<'_>, pos: [i32; 3]) -> String {
    let state = game.world.block(pos[0], pos[1], pos[2]);
    game.res.facts().get(state).name.to_string()
}

/// A player standing inside a scaffolding block, looking straight up: the eye
/// is inside the block's outline (a full block while scaffolding is in hand),
/// so the click is an inside one, its face the side the look comes from.
/// Sneaking then sends the placement up the column; a click that lost the flag
/// goes to the clicked side instead, and one that lost the face beside it.
#[test]
#[ignore = "daily"]
fn a_click_from_inside_a_scaffolding_block_places_on_the_opposite_face() {
    const COLUMN: [i32; 3] = [2, 5, 0];
    let mut resources = None;
    let mut game = server_game(&mut resources);
    game.options.dedicated = None;
    game.set_mode(GameMode::Creative);
    let scaffolding = game
        .pack
        .assets
        .registry
        .default_state("minecraft:scaffolding")
        .unwrap();
    // A supported block (stability 0), so a block may go on top of it.
    let supported = game
        .with_props(scaffolding, &[("distance", "0")])
        .expect("scaffolding has a distance property");
    game.world
        .set(COLUMN[0], COLUMN[1], COLUMN[2], supported)
        .unwrap();
    game.player = Some(
        PlayerMotion::new(
            game.motion,
            Vec3::new(
                f64::from(COLUMN[0]) + 0.5,
                f64::from(COLUMN[1]) - 1.0,
                f64::from(COLUMN[2]) + 0.5,
            ),
        )
        .unwrap(),
    );
    *game.held_mut() = Some(ItemStack::new("minecraft:scaffolding", 1));
    game.yaw = 0.0;
    game.pitch = -90.0;
    game.keys
        .press(KeyCode::ShiftLeft, horizon_client::held::Source::Keyboard);
    let hit: Hit = game.pick().expect("the eye is inside the column");
    assert_eq!(hit.pos, COLUMN, "the click is on the block the eye is in");
    assert!(game.use_hand(Some(&hit)).unwrap());
    assert_eq!(
        name(&game, [COLUMN[0], COLUMN[1] + 1, COLUMN[2]]),
        "minecraft:scaffolding",
        "inside + sneak places on the opposite of the clicked (down) face: upwards"
    );
    assert_eq!(
        name(&game, [COLUMN[0], COLUMN[1] - 1, COLUMN[2]]),
        "minecraft:air",
        "a click without the inside flag places on the clicked face: below"
    );
    assert_eq!(
        name(&game, [COLUMN[0] + 1, COLUMN[1], COLUMN[2]]),
        "minecraft:air",
        "a ray that lost the face places beside the column"
    );
}
