use super::*;
fn stack(item: &str, count: u32) -> ItemStack {
    ItemStack::new(item, count)
}
/// A head-locked stand held a helmet would refuse it in vanilla too:
/// `getEquipmentSlotForItem` falls back to MAINHAND, so the stand holds it.
#[test]
fn a_disabled_equippable_slot_falls_back_to_the_hand() {
    let mut a = ArmorStand::new(1, horizon_core::geometry::Vec3::ZERO, 0.0);
    a.show_arms = true;
    a.disabled_slots = 1 << 4; // head
    assert_eq!(usable_equipment_slot(&a, 5), 0);
    assert_eq!(usable_equipment_slot(&a, 4), 4);
    assert_eq!(usable_equipment_slot(&a, 0), 0);
    a.disabled_slots = 1 << 5; // off hand
    assert_eq!(usable_equipment_slot(&a, 1), 0);
}
#[test]
fn disabled_slots_lock_by_filter_flag() {
    let mut a = ArmorStand::new(1, horizon_core::geometry::Vec3::ZERO, 0.0);
    // Hands need arms.
    assert!(a.is_disabled(0) && a.is_disabled(1));
    a.show_arms = true;
    assert!(!a.is_disabled(0) && !a.is_disabled(1));
    // Bit 4 locks the head (equipment index 5), bit 5 the off hand (1).
    a.disabled_slots = (1 << 4) | (1 << 5);
    assert!(a.is_disabled(5) && a.is_disabled(1));
    assert!(!a.is_disabled(4) && !a.is_disabled(0));
    // Taking (+8) from a filled slot and putting (+16) into an empty one.
    a.disabled_slots = (1 << (3 + 8)) | (1 << (1 + 16));
    assert!(!a.swap_locked(4), "an empty chest slot is not take-locked");
    a.equipment[4] = Some(stack("minecraft:iron_chestplate", 1));
    assert!(a.swap_locked(4));
    assert!(a.swap_locked(2), "an empty feet slot is put-locked");
    a.equipment[2] = Some(stack("minecraft:iron_boots", 1));
    assert!(!a.swap_locked(2));
}
#[test]
fn creative_swap_gives_the_worn_piece_to_the_hand() {
    let swap = swap_with_equipment_slot(
        &stack("minecraft:diamond_chestplate", 1),
        Some(stack("minecraft:iron_chestplate", 1)),
        true,
    );
    assert_eq!(swap.worn, stack("minecraft:diamond_chestplate", 1));
    assert_eq!(swap.hand, Some(stack("minecraft:iron_chestplate", 1)));
    assert_eq!(swap.to_inventory, None);
}
#[test]
fn creative_equip_with_nothing_worn_keeps_the_hand() {
    let swap = swap_with_equipment_slot(&stack("minecraft:diamond_helmet", 1), None, true);
    assert_eq!(swap.worn, stack("minecraft:diamond_helmet", 1));
    assert_eq!(swap.hand, Some(stack("minecraft:diamond_helmet", 1)));
}
#[test]
fn survival_swap_gives_the_worn_piece_or_empties_the_hand() {
    let worn = Some(stack("minecraft:iron_boots", 1));
    let swap = swap_with_equipment_slot(&stack("minecraft:diamond_boots", 1), worn, false);
    assert_eq!(swap.hand, Some(stack("minecraft:iron_boots", 1)));
    let swap = swap_with_equipment_slot(&stack("minecraft:diamond_boots", 1), None, false);
    assert_eq!(swap.hand, None);
}
#[test]
fn a_larger_stack_wears_one_and_sends_the_old_piece_to_the_inventory() {
    // Armour stacks past one only with a max_stack_size component.
    let worn = Some(stack("minecraft:leather_helmet", 1));
    let swap = swap_with_equipment_slot(&stack("minecraft:golden_helmet", 3), worn.clone(), false);
    assert_eq!(swap.worn, stack("minecraft:golden_helmet", 1));
    assert_eq!(swap.hand, Some(stack("minecraft:golden_helmet", 2)));
    assert_eq!(swap.to_inventory, worn);
    let swap = swap_with_equipment_slot(&stack("minecraft:golden_helmet", 3), worn.clone(), true);
    assert_eq!(swap.hand, Some(stack("minecraft:golden_helmet", 3)));
    assert_eq!(swap.to_inventory, worn);
}
