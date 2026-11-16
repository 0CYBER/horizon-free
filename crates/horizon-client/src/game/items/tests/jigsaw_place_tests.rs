//! The joint a placed jigsaw starts with: `JigsawBlock.newBlockEntity` leaves
//! `JigsawBlockEntity`'s constructor default `ROLLABLE`, and only the item's
//! `block_entity_data` (`BlockItem.updateCustomBlockEntityTag` ->
//! `loadAdditional`) or a loaded tag changes it. The facing-based
//! `StructureTemplate.getDefaultJointType` is `loadAdditional`'s fallback, not
//! a placement rule.
use crate::game::{Game, Hit, native_machines::tests::server_game};
use horizon_client::{containers::BlockEntity, items::ItemStack, jigsaw_block::JointType};

/// A server's game with nobody playing, or `None` without prepared resources.
fn game() -> Option<Game<'static>> {
    std::env::var_os("HORIZON_TEST_RESOURCES")?;
    let mut game = server_game();
    game.set_mode_id(1);
    game.ui.registries.permission = 2;
    Some(game)
}

/// `/give`'s jigsaw, with a `joint` in its `block_entity_data` when given.
fn jigsaw_item(joint: Option<&str>) -> ItemStack {
    let mut stack = ItemStack::new("minecraft:jigsaw", 1);
    let Some(joint) = joint else {
        return stack;
    };
    let text =
        format!(r#"{{"minecraft:block_entity_data":{{id:"minecraft:jigsaw",joint:"{joint}"}}}}"#);
    let components = horizon_core::nbt::parse_snbt(&text).expect("snbt").0;
    let (damage, extra) = horizon_client::item_data::decode_components(Some(&components));
    stack.damage = damage;
    stack.extra = extra;
    stack
}

/// A click on the block at `pos` from the side `face`.
fn click(pos: [i32; 3], face: [i32; 3]) -> Hit {
    let mut point = [
        f64::from(pos[0]) + 0.5,
        f64::from(pos[1]) + 0.5,
        f64::from(pos[2]) + 0.5,
    ];
    for (axis, step) in face.iter().enumerate() {
        if *step != 0 {
            point[axis] = f64::from(pos[axis]) + if *step > 0 { 1.0 } else { 0.0 };
        }
    }
    Hit {
        pos,
        face,
        point,
        inside: false,
    }
}

/// The `orientation` of the block at `pos` (`JigsawBlock.getStateForPlacement`).
fn orientation(game: &Game<'_>, pos: [i32; 3]) -> String {
    let registry = game.pack.assets.registry;
    let state = game.world.block(pos[0], pos[1], pos[2]);
    registry
        .state(state)
        .unwrap()
        .properties
        .get("orientation")
        .cloned()
        .unwrap_or_default()
}

/// The `joint` of the jigsaw block entity at `pos`.
fn joint(game: &Game<'_>, pos: [i32; 3]) -> Option<JointType> {
    match game.world.block_entities.get(&(pos[0], pos[1], pos[2])) {
        Some(BlockEntity::JigsawBlock(b)) => Some(b.joint),
        _ => None,
    }
}

/// A jigsaw placed against a side faces horizontally, yet starts `rollable`
/// (the constructor's default); an item that carries `joint:"aligned"` keeps
/// it, whatever the facing.
#[test]
fn a_placed_jigsaw_keeps_its_default_or_its_items_joint() {
    let Some(mut game) = game() else {
        return;
    };
    // The north side: `FrontAndTop.NORTH_UP`, a horizontal front whose
    // `getDefaultJointType` would be ALIGNED.
    *game.held_mut() = Some(jigsaw_item(None));
    assert!(game.use_hand(Some(&click([8, 64, 8], [0, 0, -1]))).unwrap());
    assert_eq!(orientation(&game, [8, 64, 8]), "north_up");
    assert_eq!(joint(&game, [8, 64, 8]), Some(JointType::Rollable));

    // The top side is a vertical front, whose default is ROLLABLE: the item's
    // own joint must survive it.
    *game.held_mut() = Some(jigsaw_item(Some("aligned")));
    assert!(game.use_hand(Some(&click([10, 64, 8], [0, 1, 0]))).unwrap());
    assert!(orientation(&game, [10, 64, 8]).starts_with("up_"));
    assert_eq!(joint(&game, [10, 64, 8]), Some(JointType::Aligned));
}
