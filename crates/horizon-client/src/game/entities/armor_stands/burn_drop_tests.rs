use super::*;
use horizon_items::enchant::Enchantments;
#[test]
fn only_visible_non_marker_stands_are_broken_by_blasts() {
    let mut stand = ArmorStand::new(1, Vec3::new(0.5, 64.0, 0.5), 0.0);
    assert!(!spared_by_blast(&stand));
    stand.invisible = true;
    assert!(spared_by_blast(&stand));
    stand.invisible = false;
    stand.marker = true;
    assert!(spared_by_blast(&stand));
}
#[test]
fn burnt_equipment_drops_above_the_block_and_vanishing_items_do_not_drop() {
    let mut stand = ArmorStand::new(1, Vec3::new(-0.1, 4.8, 0.0), 0.0);
    let kept = ItemStack::new("minecraft:diamond_boots", 1);
    let mut vanished = ItemStack::new("minecraft:diamond_helmet", 1);
    vanished
        .data_mut()
        .enchantments
        .push(("minecraft:vanishing_curse".into(), 1));
    stand.equipment[0] = Some(kept.clone());
    stand.equipment[1] = Some(vanished);
    let definition = serde_json::json!({"effects":{"minecraft:prevent_equipment_drop":{}}});
    let enchantments = Enchantments::default()
        .with_entries([("minecraft:vanishing_curse", &definition)])
        .unwrap()
        .unwrap();
    let (pos, drops) = broken_armor_stand_equipment(stand, &enchantments);
    assert_eq!(pos, [-1, 5, 0]);
    assert_eq!(drops, vec![kept]);
}
#[test]
fn a_broken_stand_pops_body_and_saddle_after_the_other_six_slots() {
    use horizon_items::equipment::{BODY, HEAD, MAINHAND, SADDLE};
    let mut stand = ArmorStand::new(1, Vec3::new(0.5, 4.0, 0.5), 0.0);
    let stacks: Vec<_> = [SADDLE, BODY, HEAD, MAINHAND]
        .into_iter()
        .map(|slot| {
            let mut stack = ItemStack::new("minecraft:stone", 1 + slot as u32);
            stack.damage = slot as u32;
            stand.equipment[slot] = Some(stack.clone());
            (slot, stack)
        })
        .collect();
    let (_, drops) = broken_armor_stand_equipment(stand, &Enchantments::default());
    let order = |slots: [usize; 4]| {
        slots
            .map(|s| {
                stacks
                    .iter()
                    .find(|(slot, _)| *slot == s)
                    .unwrap()
                    .1
                    .clone()
            })
            .to_vec()
    };
    assert_eq!(drops, order([MAINHAND, HEAD, BODY, SADDLE]));
}
