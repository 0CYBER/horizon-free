//! `player_ender_chest` and `player_set_ender_chest_slot` against real LAN guests, through the
//! public player handle.

use super::native_guest_turn_tests::with_guests;
use mod_api::{Host, ItemStack, PlayerDataRefusal, StackWriteError, prelude::*};

fn api_item(game: &super::Game<'_>, snbt: &str) -> Option<ItemStack> {
    let host: &dyn Host = game;
    host.item_from_snbt(snbt)
}

const BOOK: &str = r#"{id:"minecraft:written_book",count:1,components:{"minecraft:written_book_content":{title:{raw:"Log"},author:"guest13",pages:[{raw:"one"}]}}}"#;

/// Ender chest slots are written and read back by number, out of the player's turn and in
/// it, components and all, and each player has their own.
#[test]
#[ignore = "daily"]
fn ender_chest_slots_are_read_and_written() {
    with_guests(|game| {
        let players = game.players().server_players();
        let (a, b) = (players[0], players[1]);
        let book = api_item(game, BOOK).unwrap();
        let pearls = api_item(game, r#"{id:"minecraft:ender_pearl",count:16}"#).unwrap();
        assert_eq!(b.ender_chest(game), Some(Vec::new()));
        assert_eq!(b.set_ender_chest_slot(game, 26, Some(book.clone())), Ok(()));
        game.as_guest(12, |game| {
            assert_eq!(
                a.set_ender_chest_slot(game, 0, Some(pearls.clone())),
                Ok(())
            );
            assert_eq!(a.ender_chest(game), Some(vec![(0, pearls.clone())]));
            assert_eq!(b.ender_chest(game), Some(vec![(26, book.clone())]));
        })
        .unwrap();
        assert_eq!(b.ender_chest(game), Some(vec![(26, book.clone())]));
        assert_eq!(a.ender_chest(game), Some(vec![(0, pearls)]));
        assert_eq!(b.set_ender_chest_slot(game, 26, None), Ok(()));
        assert_eq!(b.ender_chest(game), Some(Vec::new()));
    });
}

/// Each refusal has its own name and writes nothing: a slot past 26 (never a panic), a stack
/// the item does not allow, a dead player, and a handle whose player has left.
#[test]
#[ignore = "daily"]
fn each_ender_chest_refusal_has_its_own_name_and_writes_nothing() {
    with_guests(|game| {
        let b = game.players().server_players()[1];
        let book = api_item(game, BOOK).unwrap();
        assert_eq!(
            b.set_ender_chest_slot(game, 27, Some(book.clone())),
            Err(PlayerDataRefusal::SlotOutOfRange { slot: 27, last: 26 })
        );
        assert_eq!(
            b.set_ender_chest_slot(game, u32::MAX, None),
            Err(PlayerDataRefusal::SlotOutOfRange {
                slot: u32::MAX,
                last: 26
            })
        );
        let mut too_many = api_item(game, r#"{id:"minecraft:diamond_sword"}"#).unwrap();
        too_many.set_count(2).unwrap();
        assert_eq!(
            b.set_ender_chest_slot(game, 0, Some(too_many)),
            Err(PlayerDataRefusal::Stack(StackWriteError::CountExceeded {
                maximum: 1
            }))
        );
        assert_eq!(b.ender_chest(game), Some(Vec::new()));
        game.lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update_connection(13, |g| g.dead = true);
        assert_eq!(
            b.set_ender_chest_slot(game, 0, Some(book.clone())),
            Err(PlayerDataRefusal::Dead)
        );
        assert_eq!(b.ender_chest(game), Some(Vec::new()));
        game.lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update_connection(13, |g| g.dead = false);
        let index = game
            .lan
            .as_ref()
            .unwrap()
            .hub
            .guests
            .connection_index(13)
            .unwrap();
        game.lan.as_mut().unwrap().hub.guests.remove(index);
        assert_eq!(
            b.set_ender_chest_slot(game, 0, Some(book)),
            Err(PlayerDataRefusal::Stale)
        );
    });
}

/// A stack written to an ender chest slot the player's open ender chest menu shows replaces
/// the old one in that menu slot, and clicking the slot picks up the new stack.
#[test]
#[ignore = "daily"]
fn a_written_slot_is_what_the_open_ender_chest_menu_shows_and_clicks() {
    use horizon_client::lan::menu::Slots;
    use horizon_protocol::packets::play::{ContainerClick, Serverbound};
    with_guests(|game| {
        let b = game.players().server_players()[1];
        let old = api_item(game, r#"{id:"minecraft:ender_pearl",count:5}"#).unwrap();
        let new = api_item(game, BOOK).unwrap();
        let hub = &mut game.lan.as_mut().unwrap().hub;
        hub.open_container(
            13,
            "minecraft:generic_9x3",
            horizon_core::nbt::Tag::String("test".into()),
            (1, 64, 1),
            Slots::Ender,
            vec![None; 27],
            None,
        );
        let window = hub.guest(13).unwrap().menu.as_ref().unwrap().window;
        assert_eq!(b.set_ender_chest_slot(game, 3, Some(old)), Ok(()));
        assert_eq!(b.set_ender_chest_slot(game, 3, Some(new)), Ok(()));
        let hub = &game.lan.as_ref().unwrap().hub;
        let guest = hub.guest(13).unwrap();
        let shown = Slots::Ender.get(&game.world, guest, 3).unwrap();
        assert_eq!(shown.item.as_ref(), "minecraft:written_book");
        game.lan_guest_packet(
            13,
            Serverbound::ContainerClick(ContainerClick {
                container: window,
                state: 0,
                slot: 3,
                button: 0,
                input: 0,
                changed: Vec::new(),
                carried: None,
            }),
        );
        let guest = game.lan.as_ref().unwrap().hub.guest(13).unwrap();
        assert_eq!(
            guest.carried,
            Some(shown),
            "the click picked up the written stack"
        );
        assert_eq!(guest.ender_items[3], None, "the old stack is gone");
    });
}

/// The local player's open ender chest menu holds a copy that closing writes back over the
/// chest: a write made while it is open goes into that copy and survives the close.
#[test]
#[ignore = "daily"]
fn a_write_while_the_local_ender_chest_is_open_survives_closing_it() {
    use horizon_client::containers::BlockEntity;
    with_guests(|game| {
        let key = (1, 64, 1);
        let old = horizon_client::items::ItemStack::new("minecraft:ender_pearl", 5);
        let new = horizon_client::items::ItemStack::new("minecraft:diamond", 2);
        game.ender_items = vec![None; 27];
        game.ender_items[3] = Some(old);
        game.open_ender_chest(key);
        assert!(game.slot_set(&super::commands::items::Slot::Ender(3), Some(new.clone())));
        let Some(BlockEntity::Chest(open)) = game.world.block_entities.get(&key) else {
            panic!("the open copy")
        };
        assert_eq!(
            open[3],
            Some(new.clone()),
            "the menu shows the written stack"
        );
        game.close_ender_chest();
        assert_eq!(game.ender_items[3], Some(new), "closing keeps it");
    });
}
