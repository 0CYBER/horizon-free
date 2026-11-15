//! `ItemPart::can_grindstone_repair` in the grindstone's result and input slots, through the
//! library's installed test callbacks, which answer from the `item_parts` setting.
use super::*;
use crate::enchant::tests::{sample, sample_items};
use crate::native_client_hooks::test_hooks::{answering, install, item_asks};

const HOOK: &str = "can_grindstone_repair";

fn worn(item: &str, max: i32, damage: u32) -> ItemStack {
    let mut s = ItemStack::new(item, 1);
    s.damage = damage;
    s.data_mut().set_other(
        "minecraft:max_damage",
        Some(horizon_core::nbt::Tag::Int(max)),
    );
    s
}

/// The asks of this hook since the last call: the item and the answer it arrived with.
fn asked() -> Vec<(String, bool)> {
    item_asks()
        .into_iter()
        .filter(|(hook, _, _)| *hook == HOOK)
        .map(|(_, item, arrived)| (item, arrived))
        .collect()
}

#[test]
fn iron_swords_repair_from_vanillas_answer_and_a_false_part_leaves_no_result() {
    install();
    let (e, items) = (sample(), sample_items());
    let a = worn("minecraft:iron_sword", 250, 200);
    let b = worn("minecraft:iron_sword", 250, 100);
    asked();
    let vanilla = crate::test_resources::server_loop_context();
    let out = grindstone_result(vanilla, &e, &items, Some(&a), Some(&b)).unwrap();
    // 250 − (50 + 150 + 12): one sword at damage 38.
    assert_eq!((out.count, out.damage), (1, 38));
    // The top stack is asked once, arriving with vanilla's answer: it is damageable.
    assert_eq!(asked(), [("minecraft:iron_sword".to_owned(), true)]);
    let refused = answering(HOOK, "minecraft:iron_sword=false");
    assert!(grindstone_result(&refused, &e, &items, Some(&a), Some(&b)).is_none());
    assert_eq!(asked(), [("minecraft:iron_sword".to_owned(), true)]);
}

#[test]
fn a_granted_enchanted_book_merges_into_one_keeping_only_the_curse() {
    install();
    let mut e = sample();
    let curses = vec!["minecraft:binding_curse".into()];
    e.tags.insert("minecraft:curse".into(), curses);
    let items = sample_items();
    let book = |id: &str| {
        let mut s = ItemStack::new("minecraft:enchanted_book", 1);
        s.enchant(id, 1);
        s
    };
    let a = book("minecraft:sharpness");
    let b = book("minecraft:binding_curse");
    asked();
    // Vanilla: not damageable, and the two differ, so nothing.
    let vanilla = crate::test_resources::server_loop_context();
    assert!(grindstone_result(vanilla, &e, &items, Some(&a), Some(&b)).is_none());
    assert_eq!(asked(), [("minecraft:enchanted_book".to_owned(), false)]);
    let granted = answering(HOOK, "minecraft:enchanted_book=true");
    let out = grindstone_result(&granted, &e, &items, Some(&a), Some(&b)).unwrap();
    assert_eq!((&*out.item, out.count), ("minecraft:enchanted_book", 1));
    let curse = Arc::<str>::from("minecraft:binding_curse");
    assert_eq!(out.crafting_enchantments(), [(curse, 1)]);
    assert_eq!(out.data().unwrap().repair_cost, 1);
    // No durability is merged into an item that has none.
    assert_eq!(out.damage, 0);
    assert!(out.data().unwrap().other("minecraft:max_damage").is_none());
}

#[test]
fn a_granted_stick_is_placed_and_two_give_one() {
    install();
    let (e, items) = (sample(), sample_items());
    let stick = ItemStack::new("minecraft:stick", 1);
    let vanilla = crate::test_resources::server_loop_context();
    asked();
    assert!(!grindstone_accepts(vanilla, &items, &stick));
    assert_eq!(asked(), [("minecraft:stick".to_owned(), false)]);
    // Vanilla's other branch: two matching stacks of an item that stacks give two.
    let out = grindstone_result(vanilla, &e, &items, Some(&stick), Some(&stick)).unwrap();
    assert_eq!(out.count, 2);
    let granted = answering(HOOK, "minecraft:stick=true");
    assert!(grindstone_accepts(&granted, &items, &stick));
    let out = grindstone_result(&granted, &e, &items, Some(&stick), Some(&stick)).unwrap();
    assert_eq!((&*out.item, out.count), ("minecraft:stick", 1));
}

#[test]
fn a_lone_or_enchanted_input_is_never_asked() {
    install();
    let (e, items) = (sample(), sample_items());
    let mut sword = ItemStack::new("minecraft:diamond_sword", 1);
    sword.enchant("minecraft:sharpness", 1);
    let refused = answering(HOOK, "minecraft:diamond_sword=false");
    asked();
    // The single-input branch merges nothing, and an enchanted stack is placed as it is.
    let out = grindstone_result(&refused, &e, &items, Some(&sword), None).unwrap();
    assert!(!out.has_any_enchantments());
    assert!(grindstone_accepts(&refused, &items, &sword));
    assert!(asked().is_empty());
}
