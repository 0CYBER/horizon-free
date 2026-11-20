//! The global `dispenser_acts` hook from a real dispenser: a mod's answer for any item wins over
//! the faced block's and the item's parts, and no answer leaves vanilla's behaviour alone.
use crate::game::Game;
use horizon_client::{chunk::Chunk, items::ItemStack};
use horizon_core::geometry::Direction;
use mod_api::*;
use std::sync::{Arc, Mutex};

const DISPENSER: [i32; 3] = [5, 64, 5];

/// Empties the slot of any stack of sticks and answers nothing for other items.
struct Everything(Arc<Mutex<Vec<&'static str>>>);
struct Front(Arc<Mutex<Vec<&'static str>>>);
struct Item(Arc<Mutex<Vec<&'static str>>>);

#[mod_api::horizon_part]
impl BlockPart for Front {
    fn dispense_method(
        &self,
        _: &mut dyn ServerHost,
        _: &mut DispenseMethodEvent<'_>,
    ) -> Option<Dispensed> {
        self.0.lock().unwrap().push("block");
        None
    }
}

#[mod_api::horizon_part]
impl ItemPart for Item {
    fn dispense_item(
        &self,
        _: &mut dyn ServerHost,
        _: &mut DispenseItemEvent<'_>,
    ) -> Option<Dispensed> {
        self.0.lock().unwrap().push("item");
        Some((None, 1000))
    }
}

#[mod_api::__subscriptions]
impl Hooks for Everything {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let stone = r.block_key("minecraft:stone");
        r.block_part(stone, Front(self.0.clone()));
        for name in ["minecraft:stick", "minecraft:cobblestone"] {
            let item = r.item_key(name);
            r.item_part(item, Item(self.0.clone()));
        }
    }

    fn dispenser_acts(
        &self,
        _: &mut dyn ServerHost,
        event: &mut DispenserActsEvent<'_>,
    ) -> Option<Dispensed> {
        (mod_api::__private::item_key_id(event.stack.item()) == Some("minecraft:stick")
            && event.facing == Direction::East)
            .then_some((None, 1001))
    }
}

#[test]
#[ignore = "daily"]
fn a_mods_global_answer_replaces_vanillas_dispense_and_no_answer_keeps_it() {
    let heard = Arc::new(Mutex::new(Vec::new()));
    let mods: Mods = vec![ModEntry::new(
        "everything",
        Box::new(Everything(heard.clone())),
    )]
    .into();
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        crate::game::native_entity_refs::tests::with_game(|game: &mut Game<'_>| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.native_lifecycle
                .begin_open(mod_api::Authority::IntegratedServer);
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(6, 64, 5, stone).unwrap();
            let at = (DISPENSER[0], DISPENSER[1], DISPENSER[2]);
            assert_eq!(
                game.dispense_item(at, Direction::East, ItemStack::new("minecraft:stick", 3))
                    .unwrap(),
                None
            );
            assert!(heard.lock().unwrap().is_empty());
            assert!(game.dropped.is_empty());
            assert_eq!(
                game.dispense_item(
                    at,
                    Direction::East,
                    ItemStack::new("minecraft:cobblestone", 3)
                )
                .unwrap(),
                None
            );
            assert_eq!(*heard.lock().unwrap(), ["block", "item"]);
            assert!(game.dropped.is_empty());
            let left = game
                .dispense_item(at, Direction::East, ItemStack::new("minecraft:dirt", 3))
                .unwrap()
                .unwrap();
            assert_eq!(left.count, 2);
        });
    });
}
