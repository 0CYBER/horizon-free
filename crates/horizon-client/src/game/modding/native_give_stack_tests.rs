//! `player_give_stack` against a real LAN guest and `item_from_snbt` on the public host.

use super::{api_values::TryIntoApi, native_guest_turn_tests::with_guests};
use horizon_core::nbt::parse_snbt;
use mod_api::{Host, ItemStack, PlayerDataRefusal, prelude::*};

fn api_item(game: &super::Game<'_>, snbt: &str) -> Option<ItemStack> {
    let host: &dyn Host = game;
    host.item_from_snbt(snbt)
}

/// What a mod reads from SNBT and writes back is the compound `ItemStack.CODEC` started
/// from, through the API's own stack and back.
#[test]
#[ignore = "daily"]
fn an_snbt_item_goes_through_the_api_unchanged() {
    with_guests(|game| {
        for snbt in [
            r#"{id:"minecraft:diamond_sword",count:1,components:{"minecraft:damage":5,"minecraft:enchantments":{"minecraft:sharpness":3},"minecraft:custom_name":"Blade","minecraft:lore":["one","two"],"minecraft:custom_data":{husksync:{v:1b}}}}"#,
            r#"{id:"minecraft:stone",count:64}"#,
            r#"{id:"minecraft:potion",count:1,components:{"minecraft:potion_contents":{potion:"minecraft:strong_healing"}}}"#,
            r#"{id:"minecraft:written_book",count:1,components:{"minecraft:written_book_content":{title:{raw:"Log"},author:"guest13",pages:[{raw:"one"}]}}}"#,
        ] {
            let engine = game
                .item_from_snbt(snbt)
                .unwrap_or_else(|| panic!("{snbt}"));
            let api = api_item(game, snbt).unwrap();
            assert_eq!(
                Some(&api),
                engine.clone().try_into_api().ok().as_ref(),
                "{snbt}"
            );
            let (tag, _) = parse_snbt(snbt).unwrap();
            assert_eq!(
                horizon_items::item_nbt::item_to_nbt_with_items(&engine, game.items()),
                tag,
                "{snbt}"
            );
        }
    });
}

/// This API refuses with None: text that is not one compound, text with anything but
/// whitespace after the compound, text over 2 MiB, nesting past 512 levels, air and an item
/// the world lacks, and a count outside 1 to 99.
#[test]
#[ignore = "daily"]
fn item_from_snbt_refuses_what_is_not_one_item_compound() {
    with_guests(|game| {
        let host: &dyn Host = game;
        let count = |snbt: &str| host.item_from_snbt(snbt).map(|s| s.count());
        assert_eq!(
            count(r#"{id:"stone"}"#),
            Some(1),
            "stone is minecraft:stone"
        );
        assert_eq!(count(r#"{id:"minecraft:stone",count:99}"#), Some(99));
        assert_eq!(count(r#"{id:"minecraft:stone",count:1}"#), Some(1));
        assert_eq!(
            count(r#"{id:"minecraft:stone",count:100}"#),
            None,
            "a count past 99 is refused, not clamped"
        );
        assert_eq!(count(r#"{id:"minecraft:stone",count:0}"#), None);
        assert_eq!(count(r#"{id:"minecraft:stone",count:-1}"#), None);
        assert_eq!(count(r#"{id:"minecraft:stone",count:"5"}"#), None);
        assert_eq!(count("  {id:\"minecraft:stone\"} \n"), Some(1));
        assert_eq!(count("stone"), None, "no compound");
        assert_eq!(count("-"), None, "no item");
        assert_eq!(count(r#"[{id:"minecraft:stone"}]"#), None);
        assert_eq!(
            count(r#"{id:"minecraft:stone"} x"#),
            None,
            "text after the compound"
        );
        assert_eq!(
            count(r#"{id:"minecraft:stone"}{id:"minecraft:stone"}"#),
            None
        );
        assert_eq!(count(r#"{id:"minecraft:air",count:1}"#), None);
        assert_eq!(count(r#"{id:"nomod:thing",count:1}"#), None);
        assert_eq!(count(r#"{count:1}"#), None);
        let item = r#"{id:"minecraft:stone"}"#;
        let padded = |len: usize| format!("{item}{}", " ".repeat(len - item.len()));
        assert_eq!(count(&padded(2 * 1024 * 1024)), Some(1), "2 MiB is read");
        assert_eq!(
            count(&padded(2 * 1024 * 1024 + 1)),
            None,
            "one byte more is not"
        );
        let nested = |levels: usize| {
            format!(
                r#"{{id:"minecraft:stone",components:{{"minecraft:custom_data":{{a:{}0{}}}}}}}"#,
                "[".repeat(levels),
                "]".repeat(levels)
            )
        };
        assert_eq!(count(&nested(8)), Some(1));
        assert_eq!(count(&nested(600)), None, "nested past 512 levels");
    });
}

/// A given stack goes into the inventory where it fits, components and all; the rest is
/// thrown for the player alone, with no pickup delay.
#[test]
#[ignore = "daily"]
fn a_given_stack_fills_the_inventory_then_drops_for_its_player() {
    with_guests(|game| {
        let b = game.players().server_players()[1];
        let rock = r#"{id:"minecraft:stone",count:10,components:{"minecraft:custom_name":"Rock"}}"#;
        let named = api_item(game, rock).unwrap();
        let engine = game.item_from_snbt(rock).unwrap();
        game.lan
            .as_mut()
            .unwrap()
            .hub
            .update_guest(13, |g| {
                let dirt = Some(horizon_client::items::ItemStack::new("minecraft:dirt", 64));
                g.inventory.iter_mut().for_each(|s| *s = dirt.clone());
                g.inventory[5] = Some(horizon_client::items::ItemStack {
                    count: 60,
                    ..engine.clone()
                });
            })
            .unwrap();
        let before = game.dropped.len();
        assert_eq!(b.give_stack(game, named.clone()), Ok(()));
        let g = game.lan.as_ref().unwrap().hub.guest(13).unwrap();
        assert_eq!(g.inventory[5].as_ref().map(|s| s.count), Some(64));
        assert_eq!(game.dropped.len(), before + 1);
        let dropped = game.dropped.last().unwrap();
        assert_eq!(dropped.stack.count, 6);
        assert_eq!(dropped.stack.extra, engine.extra);
        assert_eq!(dropped.pickup_delay, 0, "no pickup delay");
        assert_eq!(
            dropped.target,
            Some(horizon_client::lan::playerdata::uuid_ints(13)),
            "only its player picks it up"
        );
        game.lan
            .as_mut()
            .unwrap()
            .hub
            .update_guest(13, |g| g.inventory[7] = None)
            .unwrap();
        assert_eq!(b.give_stack(game, named), Ok(()));
        let g = game.lan.as_ref().unwrap().hub.guest(13).unwrap();
        assert_eq!(g.inventory[7], Some(engine));
        assert_eq!(game.dropped.len(), before + 1);
    });
}

/// This API refuses a dead player with its own name and gives nothing.
#[test]
#[ignore = "daily"]
fn a_dead_player_is_given_nothing() {
    with_guests(|game| {
        let b = game.players().server_players()[1];
        let rock = api_item(game, r#"{id:"minecraft:stone",count:3}"#).unwrap();
        game.lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update_connection(13, |g| g.dead = true);
        let before = game.dropped.len();
        assert_eq!(b.give_stack(game, rock), Err(PlayerDataRefusal::Dead));
        assert_eq!(b.inventory_slots(game), Some(Vec::new()));
        assert_eq!(game.dropped.len(), before);
    });
}
