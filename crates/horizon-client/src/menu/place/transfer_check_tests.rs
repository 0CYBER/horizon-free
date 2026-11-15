//! A viewer transfer worked out without moving anything (`Menu::check_transfer`),
//! against the vanilla data the menu tests use.
use crate::menu::{
    Kind, Menu,
    place::{TransferCheck, TransferError},
    tests::{mod_crafting_table, stack, with_menu},
};

fn missing(cells: Vec<usize>) -> TransferCheck {
    TransferCheck {
        result: Err(TransferError::Missing),
        missing: cells,
    }
}

/// The check answers as the transfer would and names the cells nothing fills.
#[test]
fn a_check_answers_as_the_transfer_would() {
    with_menu(Kind::Crafting, |env, menu| {
        // A stick is two planks, one above the other.
        assert_eq!(
            menu.check_transfer(env, "minecraft:stick"),
            missing(vec![0, 1])
        );
        menu.set_player_item(4, stack("oak_planks", 1));
        assert_eq!(
            menu.check_transfer(env, "minecraft:stick"),
            missing(vec![1])
        );
        menu.set_player_item(5, stack("birch_planks", 1));
        let ok = menu.check_transfer(env, "minecraft:stick");
        assert_eq!(ok.result, Ok(()));
        assert!(ok.missing.is_empty());
        // A crafting table is a 2x2 of planks: the last two lack.
        assert_eq!(
            menu.check_transfer(env, "minecraft:crafting_table"),
            missing(vec![2, 3])
        );
        // A recipe the server doesn't know is not into this menu.
        assert_eq!(
            menu.check_transfer(env, "minecraft:no_such_recipe").result,
            Err(TransferError::Unavailable)
        );
        assert_eq!(
            menu.check_transfer(env, "minecraft:stick"),
            menu.clone().check_transfer(env, "minecraft:stick")
        );
        let mut transferred = menu.clone();
        assert_eq!(
            transferred.transfer_recipe(env, "minecraft:stick", false),
            ok.result
        );
    });
    // The grid's items have nowhere to go: no room, as the transfer says.
    with_menu(Kind::Crafting, |env, menu| {
        menu.set_player_item(0, stack("oak_planks", 2));
        for i in 1..36 {
            menu.set_player_item(i, stack("dirt", 64));
        }
        menu.set_slot(env, 5, stack("cobblestone", 1));
        assert_eq!(
            menu.check_transfer(env, "minecraft:stick").result,
            Err(TransferError::NoRoom)
        );
        assert_eq!(
            menu.clone().transfer_recipe(env, "minecraft:stick", false),
            Err(TransferError::NoRoom)
        );
    });
}

/// A check counts a mod grid's sources and leaves the slots, the sources and
/// the drops as they were, whatever it answers.
#[test]
fn a_check_counts_sources_and_changes_nothing() {
    let station = &mut Menu::new(mod_crafting_table());
    with_menu(Kind::Inventory, |env, _| {
        let state = |m: &Menu| {
            (
                (0..m.slot_count())
                    .map(|s| m.slot(s).cloned())
                    .collect::<Vec<_>>(),
                m.sources.clone(),
                m.drops.len(),
                m.dropped.len(),
            )
        };
        station.set_player_item(0, stack("oak_planks", 1));
        station.sources = vec![vec![None, stack("oak_planks", 3)]];
        let before = state(station);
        assert_eq!(
            station
                .check_transfer(env, "minecraft:crafting_table")
                .result,
            Ok(())
        );
        assert_eq!(state(station), before);
        station.sources = vec![vec![stack("dirt", 5)]];
        let before = state(station);
        assert_eq!(
            station.check_transfer(env, "minecraft:stick"),
            missing(vec![1])
        );
        assert_eq!(state(station), before);
    });
}
