//! `player_inventory_slots` and `player_set_inventory_slot` against real LAN guests, through the
//! public player handle.

use super::native_guest_turn_tests::with_guests;
use mod_api::{Host, HostTransaction, ItemStack, PlayerDataRefusal, StackWriteError, prelude::*};

fn api_item(game: &super::Game<'_>, snbt: &str) -> Option<ItemStack> {
    let host: &dyn Host = game;
    host.item_from_snbt(snbt)
}

fn stack(game: &super::Game<'_>, snbt: &str) -> ItemStack {
    api_item(game, snbt).unwrap_or_else(|| panic!("{snbt}"))
}

fn guest<'a>(game: &'a super::Game<'_>, conn: u32) -> &'a horizon_client::lan::guest::Guest {
    game.lan.as_ref().unwrap().hub.guest(conn).unwrap()
}

const BLADE: &str = r#"{id:"minecraft:diamond_sword",count:1,components:{"minecraft:damage":5,"minecraft:custom_name":"Blade","minecraft:custom_data":{husksync:1b}}}"#;

/// Slots go by `Inventory`'s numbers both ways, out of the player's turn and in it; the
/// inventory menu has the changed slots left to send, and a stack with components comes back
/// as it went in.
#[test]
#[ignore = "daily"]
fn inventory_slots_are_read_and_written_by_vanillas_numbers() {
    with_guests(|game| {
        let (a, b) = {
            let players = game.players().server_players();
            (players[0], players[1])
        };
        let blade = stack(game, BLADE);
        let boots = stack(game, r#"{id:"minecraft:iron_boots",count:1}"#);
        let shield = stack(game, r#"{id:"minecraft:shield",count:1}"#);
        assert_eq!(b.inventory_slots(game), Some(Vec::new()));
        assert_eq!(b.set_inventory_slot(game, 8, Some(blade.clone())), Ok(()));
        assert_eq!(b.set_inventory_slot(game, 36, Some(boots.clone())), Ok(()));
        assert_eq!(b.set_inventory_slot(game, 40, Some(shield.clone())), Ok(()));
        let g = guest(game, 13);
        assert_eq!(
            g.inventory[8].as_ref().map(|s| s.item.as_ref()),
            Some("minecraft:diamond_sword")
        );
        assert_eq!(
            g.armor[3].as_ref().map(|s| s.item.as_ref()),
            Some("minecraft:iron_boots"),
            "36 is the feet"
        );
        assert_eq!(
            g.offhand.as_ref().map(|s| s.item.as_ref()),
            Some("minecraft:shield")
        );
        // The inventory menu's hotbar 8 is slot 44, feet 8, off hand 45: unsent yet.
        for menu in [44, 8, 45] {
            assert_ne!(g.menu_slot(menu), g.menu_sent[menu], "menu slot {menu}");
        }
        assert_eq!(
            b.inventory_slots(game),
            Some(vec![(8, blade.clone()), (36, boots), (40, shield)])
        );
        assert_eq!(
            api_item(game, BLADE).as_ref(),
            Some(&blade),
            "damage, name and custom data survive the round trip"
        );
        // In another player's turn the first one is displaced, and still reached.
        game.as_guest(12, |game| {
            assert_eq!(a.set_inventory_slot(game, 39, Some(blade.clone())), Ok(()));
            assert_eq!(a.inventory_slots(game), Some(vec![(39, blade.clone())]));
            assert_eq!(b.set_inventory_slot(game, 8, None), Ok(()));
        })
        .unwrap();
        assert_eq!(guest(game, 13).inventory[8], None);
        assert_eq!(
            a.inventory_slots(game),
            Some(vec![(39, blade)]),
            "39 is the head"
        );
    });
}

/// Each refusal has its own name and writes nothing: a slot past 42 (never a panic), a stack
/// the item does not allow, a dead player, and a handle whose player has left.
#[test]
#[ignore = "daily"]
fn each_inventory_refusal_has_its_own_name_and_writes_nothing() {
    with_guests(|game| {
        let b = game.players().server_players()[1];
        let rock = stack(game, r#"{id:"minecraft:stone",count:1}"#);
        assert_eq!(
            b.set_inventory_slot(game, 43, Some(rock.clone())),
            Err(PlayerDataRefusal::SlotOutOfRange { slot: 43, last: 42 })
        );
        assert_eq!(
            b.set_inventory_slot(game, u32::MAX, None),
            Err(PlayerDataRefusal::SlotOutOfRange {
                slot: u32::MAX,
                last: 42
            })
        );
        let mut too_many = rock.clone();
        too_many.set_count(65).unwrap();
        assert_eq!(
            b.set_inventory_slot(game, 0, Some(too_many)),
            Err(PlayerDataRefusal::Stack(StackWriteError::CountExceeded {
                maximum: 64
            }))
        );
        assert_eq!(b.inventory_slots(game), Some(Vec::new()));
        game.lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update_connection(13, |g| g.dead = true);
        assert_eq!(
            b.set_inventory_slot(game, 0, Some(rock.clone())),
            Err(PlayerDataRefusal::Dead)
        );
        assert_eq!(
            b.inventory_slots(game),
            Some(Vec::new()),
            "a dead player is read, not written"
        );
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
            b.set_inventory_slot(game, 0, Some(rock)),
            Err(PlayerDataRefusal::Stale)
        );
        assert_eq!(b.inventory_slots(game), None);
    });
}

/// A stack written to a slot the open inventory menu shows replaces the old one in that
/// menu slot, and clicking the slot picks up the new stack: no stale copy of the old one.
#[test]
#[ignore = "daily"]
fn a_written_slot_is_what_the_open_inventory_menu_shows_and_clicks() {
    use horizon_protocol::packets::play::{ContainerClick, Serverbound};
    with_guests(|game| {
        let b = game.players().server_players()[1];
        let old = stack(game, r#"{id:"minecraft:stone",count:5}"#);
        let new = stack(game, BLADE);
        assert_eq!(b.set_inventory_slot(game, 8, Some(old)), Ok(()));
        assert_eq!(b.set_inventory_slot(game, 8, Some(new)), Ok(()));
        // The inventory menu's hotbar 8 is menu slot 44.
        let shown = guest(game, 13).menu_slot(44).unwrap();
        assert_eq!(shown.item.as_ref(), "minecraft:diamond_sword");
        assert_eq!(shown.count, 1);
        game.lan_guest_packet(
            13,
            Serverbound::ContainerClick(ContainerClick {
                container: 0,
                state: 0,
                slot: 44,
                button: 0,
                input: 0,
                changed: Vec::new(),
                carried: None,
            }),
        );
        let g = guest(game, 13);
        assert_eq!(
            g.carried,
            Some(shown),
            "the click picked up the written stack"
        );
        assert_eq!(g.menu_slot(44), None);
        assert_eq!(g.inventory[8], None, "the old stack is gone");
    });
}

#[test]
#[ignore = "daily"]
fn inventory_write_aborts() {
    with_guests(|game| {
        let player = game.players().server_players()[1];
        let new = stack(game, r#"{id:"minecraft:iron_boots",count:1}"#);
        let old = stack(game, r#"{id:"minecraft:stone",count:64}"#);
        player.set_inventory_slot(game, 36, Some(old)).unwrap();
        let before = player.inventory_slots(game);
        let _: Result<(), ()> = game.transaction(|host, _| {
            player.set_inventory_slot(host, 36, Some(new)).unwrap();
            Err(())
        });
        assert_eq!(player.inventory_slots(game), before);
    });
}

#[test]
#[ignore = "daily"]
fn ender_write_aborts() {
    with_guests(|game| {
        let player = game.players().server_players()[1];
        let new = stack(game, r#"{id:"minecraft:iron_boots",count:1}"#);
        let old = stack(game, r#"{id:"minecraft:stone",count:64}"#);
        player.set_ender_chest_slot(game, 0, Some(old)).unwrap();
        let before = player.ender_chest(game);
        let _: Result<(), ()> = game.transaction(|host, _| {
            player.set_ender_chest_slot(host, 0, Some(new)).unwrap();
            Err(())
        });
        assert_eq!(player.ender_chest(game), before);
    });
}

#[test]
#[ignore = "daily"]
fn data_write_aborts() {
    with_guests(|game| {
        let player = game.players().server_players()[1];
        let mut data = horizon_core::nbt::Compound::new();
        data.insert("old".into(), horizon_core::nbt::Tag::Int(7));
        player.set_persistent_data(game, data).unwrap();
        let before = player.persistent_data(game);
        let _: Result<(), ()> = game.transaction(|host, _| {
            player
                .set_persistent_data(host, Default::default())
                .unwrap();
            Err(())
        });
        assert_eq!(player.persistent_data(game), before);
    });
}

#[test]
#[ignore = "daily"]
fn give_write_aborts() {
    with_guests(|game| {
        let player = game.players().server_players()[1];
        let new = stack(game, r#"{id:"minecraft:iron_boots",count:1}"#);
        let old = stack(game, r#"{id:"minecraft:stone",count:64}"#);
        // An empty destination exercises rollback of a successful insertion, not only
        // suppression of the overflow drop from a completely full inventory.
        assert_eq!(player.inventory_slots(game), Some(Vec::new()));
        let drops = game.dropped.len();
        let _: Result<(), ()> = game.transaction(|host, _| {
            player.give_stack(host, new.clone()).unwrap();
            assert_eq!(player.inventory_slots(host), Some(vec![(0, new.clone())]));
            Err(())
        });
        assert_eq!(player.inventory_slots(game), Some(Vec::new()));
        assert_eq!(game.dropped.len(), drops);
        for slot in 0..36 {
            player
                .set_inventory_slot(game, slot, Some(old.clone()))
                .unwrap();
        }
        let before = player.inventory_slots(game);
        let drops = game.dropped.len();
        let _: Result<(), ()> = game.transaction(|host, _| {
            player.give_stack(host, new).unwrap();
            Err(())
        });
        assert_eq!(player.inventory_slots(game), before);
        assert_eq!(game.dropped.len(), drops);
    });
}

#[test]
#[ignore = "daily"]
fn equipment_effects_wait_for_root() {
    with_guests(|game| {
        let player = game.players().server_players()[1];
        let new = stack(game, r#"{id:"minecraft:iron_boots",count:1}"#);
        game.audio.capture = Some(Vec::new());
        let _: Result<(), ()> = game.transaction(|host, _| {
            host.transaction(|host, _| {
                player
                    .set_inventory_slot(host, 36, Some(new.clone()))
                    .unwrap();
                Ok::<(), ()>(())
            })?;
            Err(())
        });
        assert!(game.audio.capture.as_ref().unwrap().is_empty());
        let inner = stack(game, r#"{id:"minecraft:diamond_helmet",count:1}"#);
        let _: Result<(), ()> = game.transaction(|host, _| {
            player
                .set_inventory_slot(host, 36, Some(new.clone()))
                .unwrap();
            let outer = player.inventory_slots(host);
            let aborted: Result<(), ()> = host.transaction(|host, _| {
                player.set_inventory_slot(host, 39, Some(inner)).unwrap();
                Err(())
            });
            assert_eq!(aborted, Err(()));
            assert_eq!(player.inventory_slots(host), outer);
            Ok(())
        });
        assert_eq!(player.inventory_slots(game), Some(vec![(36, new)]));
        assert_eq!(game.audio.capture.as_ref().unwrap().len(), 1);
    });
}
