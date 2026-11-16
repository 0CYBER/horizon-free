#![cfg(test)]
//! `ItemPart::can_grindstone_repair`, `is_combine_repairable` and
//! `can_fit_inside_container_items` through the game's callbacks: the parts of a stack's item
//! are asked with the game's loop context, starting from vanilla's answer, and other items are
//! never asked. The test installs process-wide parts: run it alone with the prepared resources
//! (`HORIZON_TEST_RESOURCES`).
use super::*;
use horizon_client::items::ItemStack;
use mod_api::{
    CanFitInsideContainerItemsEvent, CanGrindstoneRepairEvent, Hooks, IsCombineRepairableEvent,
    ItemPart, LoopContext, ModEntry, Registrar, RegistrationContext,
};
use std::sync::{Arc, Mutex};

/// A part of `item` that answers `answer` to all three hooks and records each ask: the hook
/// and the answer it arrived with.
#[derive(Clone)]
struct Part {
    item: &'static str,
    answer: bool,
    asked: Arc<Mutex<Vec<(&'static str, &'static str, bool)>>>,
}

impl Part {
    fn ask(&self, hook: &'static str, value: &mut bool) {
        self.asked.lock().unwrap().push((self.item, hook, *value));
        *value = self.answer;
    }
}

#[mod_api::horizon_part]
impl ItemPart for Part {
    fn can_grindstone_repair(&self, _: &dyn LoopContext, event: &mut CanGrindstoneRepairEvent<'_>) {
        self.ask("grindstone", &mut event.can_repair);
    }
    fn is_combine_repairable(&self, _: &dyn LoopContext, event: &mut IsCombineRepairableEvent<'_>) {
        self.ask("combine", &mut event.repairable);
    }
    fn can_fit_inside_container_items(
        &self,
        _: &dyn LoopContext,
        event: &mut CanFitInsideContainerItemsEvent<'_>,
    ) {
        self.ask("fits", &mut event.fits);
    }
}

struct Mod(Vec<Part>);

#[mod_api::__subscriptions]
impl Hooks for Mod {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        for part in &self.0 {
            let key = r.item_key(part.item);
            r.item_part(key, part.clone());
        }
    }
}

fn worn(game: &Game<'_>, item: &str, damage: u32) -> ItemStack {
    let mut stack = ItemStack::new(item, 1);
    stack.damage = damage;
    assert!(horizon_client::stations::is_damageable(
        game.items(),
        &stack
    ));
    stack
}

#[test]
#[ignore = "daily"]
fn item_loop_parts_start_from_vanilla_and_decide_the_menus_answers() {
    let asked = Arc::new(Mutex::new(Vec::new()));
    let parts = [
        ("minecraft:iron_sword", false),
        ("minecraft:stick", true),
        ("minecraft:shulker_box", true),
        ("minecraft:diamond", false),
    ]
    .map(|(item, answer)| Part {
        item,
        answer,
        asked: asked.clone(),
    })
    .to_vec();
    let mods: mod_api::Mods = vec![ModEntry::new("item_menu", Box::new(Mod(parts)))].into();
    mod_platform::testing::with_mods(mods.clone(), || {
        crate::game::api_values::install_parts(&mods).unwrap();
        crate::game::native_entity_refs::tests::with_game(|game| {
            game.resolve_parts().unwrap();
            let context = &*game.loop_context;
            let take = || std::mem::take(&mut *asked.lock().unwrap());
            // The grindstone: iron swords (250) at damage 200 and 100 would give one at 38;
            // the part's `false` takes the other branch, which gives nothing for a sword.
            let enchantments = &game.stations().enchantments;
            let (a, b) = (
                worn(game, "minecraft:iron_sword", 200),
                worn(game, "minecraft:iron_sword", 100),
            );
            let result = horizon_client::stations::grindstone_result(
                context,
                enchantments,
                game.items(),
                Some(&a),
                Some(&b),
            );
            assert!(result.is_none());
            assert_eq!(take(), [("minecraft:iron_sword", "grindstone", true)]);
            // A diamond sword has no part: it repairs as in vanilla, and nobody is asked.
            let (a, b) = (
                worn(game, "minecraft:diamond_sword", 1000),
                worn(game, "minecraft:diamond_sword", 1000),
            );
            let result = horizon_client::stations::grindstone_result(
                context,
                enchantments,
                game.items(),
                Some(&a),
                Some(&b),
            );
            assert!(result.is_some());
            assert!(take().is_empty());
            // The crafting grid's combine: the callback hands the stick's `true` back, from
            // vanilla's `false` (the grid then refuses it: a stick has no durability).
            let stick = ItemStack::new("minecraft:stick", 1);
            let mut repairable = false;
            crate::game::native_item_menu_parts::is_combine_repairable(
                context,
                &stick,
                &mut repairable,
            );
            assert!(repairable);
            assert_eq!(take(), [("minecraft:stick", "combine", false)]);
            // Container items: the diamond is kept out, a shulker box is let in.
            let fits = |item: &str| {
                horizon_client::menu::fits_inside_container_items(context, &ItemStack::new(item, 1))
            };
            assert!(!fits("minecraft:diamond"));
            assert!(fits("minecraft:shulker_box"));
            assert!(fits("minecraft:emerald"));
            assert_eq!(
                take(),
                [
                    ("minecraft:diamond", "fits", true),
                    ("minecraft:shulker_box", "fits", false),
                ]
            );
        });
    });
}
