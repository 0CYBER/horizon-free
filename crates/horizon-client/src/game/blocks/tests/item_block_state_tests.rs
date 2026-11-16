//! `BlockItem.updateBlockStateFromTag` writes the state its item's
//! `block_state` component asks for with `level.setBlock(pos, modifiedState, 2)`:
//! the clients are told of the change, the neighbours are not, so a state that
//! starts providing a signal powers nothing until something else updates it.
use crate::game::{Game, Hit, game_tests::tests::server_game};
use horizon_client::{command::args::GameMode, items::ItemStack};
use horizon_core::nbt::{Compound, JavaString, Tag};

/// A stack of `item` whose `minecraft:block_state` component holds `props`.
fn with_block_state(item: &str, props: &[(&str, &str)]) -> ItemStack {
    let mut stack = ItemStack::new(item, 1);
    let mut map = Compound::new();
    for (key, value) in props {
        map.insert(
            JavaString::from(*key),
            Tag::String(JavaString::from(*value)),
        );
    }
    stack
        .data_mut()
        .set_other("minecraft:block_state", Some(Tag::Compound(map)));
    stack
}

/// One property of the block at `pos`.
fn property(game: &Game<'_>, pos: [i32; 3], key: &str) -> Option<String> {
    let state = game.world.block(pos[0], pos[1], pos[2]);
    game.pack
        .assets
        .registry
        .state(state)
        .and_then(|s| s.properties.get(key).cloned())
}

/// The component's write runs with flag 2, so the powered lever it leaves does
/// not light the lamp beside it; the ordinary placement that follows (flag 11)
/// does, which is what tells the two writes apart.
#[test]
#[ignore = "daily"]
fn a_block_state_component_does_not_update_neighbours() {
    const LAMP: [i32; 3] = [2, 4, 0];
    const BESIDE: [i32; 3] = [2, 4, 1];
    let mut resources = None;
    let mut game = server_game(&mut resources);
    game.options.dedicated = None;
    game.set_mode(GameMode::Creative);
    let lamp = game
        .pack
        .assets
        .registry
        .default_state("minecraft:redstone_lamp")
        .unwrap();
    game.world.set(LAMP[0], LAMP[1], LAMP[2], lamp).unwrap();
    // The lever goes onto the lamp's west face: a powered lever powers every
    // neighbour, so only the write's flags decide whether the lamp lights.
    let hit = Hit {
        pos: LAMP,
        face: [-1, 0, 0],
        point: [f64::from(LAMP[0]), f64::from(LAMP[1]) + 0.5, 0.5],
        inside: false,
    };
    *game.held_mut() = Some(with_block_state("minecraft:lever", &[("powered", "true")]));
    assert!(game.use_hand(Some(&hit)).unwrap());
    let lever = [LAMP[0] - 1, LAMP[1], LAMP[2]];
    assert_eq!(
        property(&game, lever, "powered").as_deref(),
        Some("true"),
        "the component's powered lever is what was placed"
    );
    assert_eq!(
        property(&game, LAMP, "lit").as_deref(),
        Some("false"),
        "updateBlockStateFromTag notifies no neighbour, so the lamp stays dark"
    );
    // An ordinary placement (BlockItem.place's flag 11) does notify: the lamp
    // lights from the redstone block put beside it.
    let beside = Hit {
        pos: LAMP,
        face: [0, 0, 1],
        point: [
            f64::from(LAMP[0]) + 0.5,
            f64::from(LAMP[1]) + 0.5,
            f64::from(LAMP[2]) + 1.0,
        ],
        inside: false,
    };
    *game.held_mut() = Some(ItemStack::new("minecraft:redstone_block", 1));
    assert!(game.use_hand(Some(&beside)).unwrap());
    let placed = game.world.block(BESIDE[0], BESIDE[1], BESIDE[2]);
    assert_eq!(
        &*game.res.facts().get(placed).name,
        "minecraft:redstone_block",
        "a redstone block was placed beside the lamp"
    );
    assert_eq!(
        property(&game, LAMP, "lit").as_deref(),
        Some("true"),
        "an ordinary placement still notifies its neighbours"
    );
}
