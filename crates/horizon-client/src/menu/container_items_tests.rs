//! `ItemPart::can_fit_inside_container_items` at its one entry point, which a shulker box's
//! slots, a bundle and a mod's item slots ask, through the library's installed test
//! callbacks, which answer from the `item_parts` setting.
use super::*;
use crate::bundle::Contents;
use crate::items::Items;
use crate::menu::{Kind, Menu};
use crate::native_client_hooks::test_hooks::{answering, install, item_asks};

const HOOK: &str = "can_fit_inside_container_items";

/// The asks of this hook since the last call: the item and the answer it arrived with.
fn asked() -> Vec<(String, bool)> {
    item_asks()
        .into_iter()
        .filter(|(hook, _, _)| *hook == HOOK)
        .map(|(_, item, arrived)| (item, arrived))
        .collect()
}

/// How many of `stack` an empty bundle takes, then a second `stack` into what is left, as a
/// carried bundle clicked on a slot does (`canItemBeInBundle`, then `tryInsert`).
fn into_bundle(context: &LoopContext, stack: &ItemStack) -> (u32, u32) {
    let items = Items::for_tests(Default::default());
    let mut contents = Contents::from_items(&items, Vec::new());
    let mut insert = |stack: &ItemStack| {
        let fits = can_be_in_bundle(context, stack);
        contents.try_insert(&items, &mut stack.clone(), fits)
    };
    let first = insert(stack);
    (first, insert(stack))
}

/// Whether slot 0 of a shulker box menu takes `stack` (`ShulkerBoxSlot.mayPlace`).
fn shulker_slot_takes(context: &LoopContext, stack: &ItemStack) -> bool {
    let menu = Menu::new(Kind::Shulker);
    let mut takes = None;
    crate::menu::tests::bare_env_in(context, |env| takes = Some(menu.may_place(env, 0, stack)));
    takes.unwrap()
}

#[test]
fn every_stack_but_a_shulker_box_fits_and_each_ask_starts_from_vanilla() {
    install();
    let vanilla = crate::test_resources::server_loop_context();
    let diamond = ItemStack::new("minecraft:diamond", 5);
    let shulker = ItemStack::new("minecraft:red_shulker_box", 1);
    asked();
    assert!(fits_inside_container_items(vanilla, &diamond));
    assert!(!fits_inside_container_items(vanilla, &shulker));
    assert_eq!(
        asked(),
        [
            ("minecraft:diamond".to_owned(), true),
            ("minecraft:red_shulker_box".to_owned(), false),
        ]
    );
    assert!(shulker_slot_takes(vanilla, &diamond));
    assert!(!shulker_slot_takes(vanilla, &shulker));
    asked();
    // An empty stack is never in a bundle, and isn't asked.
    assert!(!can_be_in_bundle(
        vanilla,
        &ItemStack::new("minecraft:diamond", 0)
    ));
    assert!(asked().is_empty());
}

#[test]
fn a_false_part_keeps_diamonds_out_of_bundles_and_shulker_boxes() {
    install();
    let refused = answering(HOOK, "minecraft:diamond=false");
    let diamond = ItemStack::new("minecraft:diamond", 5);
    assert!(!fits_inside_container_items(&refused, &diamond));
    assert_eq!(into_bundle(&refused, &diamond), (0, 0));
    assert!(!shulker_slot_takes(&refused, &diamond));
    // Another item keeps vanilla's answer.
    let stone = ItemStack::new("minecraft:stone", 1);
    assert!(shulker_slot_takes(&refused, &stone));
}

#[test]
fn a_true_part_lets_one_shulker_box_fill_a_bundle_and_nest() {
    install();
    let shulker = ItemStack::new("minecraft:shulker_box", 1);
    let vanilla = crate::test_resources::server_loop_context();
    assert_eq!(into_bundle(vanilla, &shulker), (0, 0));
    let granted = answering(HOOK, "minecraft:shulker_box=true");
    // A shulker box weighs the whole bundle: the second is refused for room.
    assert_eq!(into_bundle(&granted, &shulker), (1, 0));
    assert!(shulker_slot_takes(&granted, &shulker));
}
