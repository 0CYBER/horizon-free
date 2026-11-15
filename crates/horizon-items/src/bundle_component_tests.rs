//! `BundleItem` acts on the item **and** its `bundle_contents` component;
//! `BundleContents.getWeight` on the component alone.
use super::*;
use horizon_core::nbt::Tag;

fn removed(item: &str) -> ItemStack {
    let mut stack = ItemStack::new(item, 1);
    stack.data_mut().other.push((
        "!minecraft:bundle_contents".into(),
        Tag::compound([] as [(&str, Tag); 0]),
    ));
    stack
}

fn holding(item: &str, inside: &[(&str, u32)]) -> ItemStack {
    let mut stack = ItemStack::new(item, 1);
    stack.data_mut().bundle_contents = inside
        .iter()
        .map(|(item, n)| ItemStack::new(item, *n))
        .collect();
    stack
}

#[test]
fn only_a_bundle_with_the_component_acts_as_one() {
    let Some(items) = crate::items::prepared_items() else {
        return;
    };
    assert!(Contents::of(&items, &ItemStack::new("minecraft:bundle", 1)).is_some());
    assert!(
        Contents::of(
            &items,
            &holding("minecraft:bundle", &[("minecraft:stone", 1)])
        )
        .is_some()
    );
    // `bundle[!bundle_contents]`: `self.get(BUNDLE_CONTENTS) == null`.
    assert!(Contents::of(&items, &removed("minecraft:bundle")).is_none());
    assert!(tooltip_layout(&items, &removed("minecraft:bundle"), 0.0, 0.0, 96.0, 2).is_none());
    // `chest[bundle_contents=[...]]`: no `BundleItem` overrides.
    let chest = holding("minecraft:chest", &[("minecraft:stone", 1)]);
    assert!(Contents::of(&items, &chest).is_none());
    assert!(tooltip_layout(&items, &chest, 0.0, 0.0, 96.0, 2).is_none());
}

#[test]
fn a_bundle_without_the_component_weighs_as_a_plain_item() {
    let Some(items) = crate::items::prepared_items() else {
        return;
    };
    // getWeight: no component, no bees, so 1 / getMaxStackSize (1): full.
    let inside = Contents::from_items(&items, vec![removed("minecraft:bundle")]);
    assert_eq!(inside.weight_fraction(), 1.0);
    // An empty bundle still weighs 1/16, and the component on another item
    // weighs what it holds plus 1/16: 32 stone (1/64 each) is 1/2.
    let inside = Contents::from_items(&items, vec![ItemStack::new("minecraft:bundle", 1)]);
    assert_eq!(inside.weight_fraction(), 1.0 / 16.0);
    let chest = holding("minecraft:chest", &[("minecraft:stone", 32)]);
    let inside = Contents::from_items(&items, vec![chest]);
    assert_eq!(inside.weight_fraction(), 0.5 + 1.0 / 16.0);
}
