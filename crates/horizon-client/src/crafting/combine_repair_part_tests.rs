//! `ItemPart::is_combine_repairable` in `RepairItemRecipe.canCombine`, through the library's
//! installed test callbacks, which answer from the `item_parts` setting.
use super::*;
use crate::native_client_hooks::test_hooks::{answering, install, item_asks};

const HOOK: &str = "is_combine_repairable";

fn worn(item: &str, max: i32, damage: u32) -> ItemStack {
    let mut s = ItemStack::new(item, 1);
    s.damage = damage;
    s.data_mut()
        .set_other("minecraft:max_damage", Some(Tag::Int(max)));
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

/// The grid's repair of `a` and `b` in `context`: its damage, or None when they don't combine.
fn repaired(context: &LoopContext, a: &ItemStack, b: &ItemStack) -> Option<u32> {
    let recipes = Recipes::empty();
    let ctx = Context {
        recipes: &recipes,
        context,
        map_scale: &|_| None,
    };
    let stacks = [a, b];
    pair(&ctx, &stacks)?;
    assemble(&ctx, &stacks).map(|out| out.damage)
}

#[test]
fn both_stacks_are_asked_from_vanillas_answer_and_a_false_part_refuses() {
    install();
    let vanilla = crate::test_resources::server_loop_context();
    let iron = [
        worn("minecraft:iron_sword", 250, 200),
        worn("minecraft:iron_sword", 250, 100),
    ];
    let gold = [
        worn("minecraft:golden_sword", 32, 20),
        worn("minecraft:golden_sword", 32, 10),
    ];
    asked();
    // 250 − (50 + 150 + 12), and 32 − (12 + 22 + 1) floored at 0.
    assert_eq!(repaired(vanilla, &iron[0], &iron[1]), Some(38));
    assert_eq!(repaired(vanilla, &gold[0], &gold[1]), Some(0));
    let sword = ("minecraft:iron_sword".to_owned(), true);
    let golden = ("minecraft:golden_sword".to_owned(), true);
    // `pair` asks both stacks, and `assemble` matches the pair again.
    let both = |asked: &(String, bool)| [asked.clone(), asked.clone()];
    let expected = [both(&sword), both(&sword), both(&golden), both(&golden)];
    assert_eq!(asked(), expected.concat());
    let refused = answering(HOOK, "minecraft:iron_sword=false");
    assert_eq!(repaired(&refused, &iron[0], &iron[1]), None);
    // The first stack's `false` already decides it.
    assert_eq!(asked(), [sword]);
    assert_eq!(repaired(&refused, &gold[0], &gold[1]), Some(0));
}

#[test]
fn a_true_for_a_stack_with_no_durability_is_refused() {
    install();
    let stick = ItemStack::new("minecraft:stick", 1);
    let vanilla = crate::test_resources::server_loop_context();
    asked();
    assert_eq!(repaired(vanilla, &stick, &stick), None);
    assert_eq!(asked(), [("minecraft:stick".to_owned(), false)]);
    let granted = answering(HOOK, "minecraft:stick=true");
    assert_eq!(repaired(&granted, &stick, &stick), None);
}

#[test]
fn stacks_of_two_items_or_counts_are_never_asked() {
    install();
    let granted = answering(HOOK, "minecraft:iron_sword=true");
    let sword = worn("minecraft:iron_sword", 250, 10);
    let mut two = sword.clone();
    two.count = 2;
    asked();
    assert_eq!(repaired(&granted, &sword, &two), None);
    let golden = worn("minecraft:golden_sword", 32, 1);
    assert_eq!(repaired(&granted, &sword, &golden), None);
    assert!(asked().is_empty());
}
