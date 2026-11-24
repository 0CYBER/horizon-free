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
fn a_saved_bundle_releases_stacks_against_the_world_prototypes() {
    let ore = "minecraft:iron_ore";
    let mut items = Items::empty();
    items.items.insert(
        "minecraft:bundle".into(),
        crate::items::ItemInfo {
            bundle: true,
            max_stack: 1,
            ..Default::default()
        },
    );
    items.items.insert(
        ore.into(),
        crate::items::ItemInfo {
            max_stack: 64,
            components: vec!["minecraft:max_stack_size".into()],
            ..Default::default()
        },
    );
    items.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
        ore.to_owned(),
        [("minecraft:max_stack_size".into(), Tag::Int(64))]
            .into_iter()
            .collect(),
    )]));
    let bundle = crate::item_data::parse_stack(
        r#"{id:"minecraft:bundle",components:{"minecraft:bundle_contents":[{id:"minecraft:iron_ore",count:3,components:{"minecraft:max_stack_size":64}},{id:"minecraft:iron_ore",components:{"minecraft:max_stack_size":16}}]}}"#,
    )
    .unwrap()
    .unwrap();

    let mut contents = Contents::of(&items, &bundle).unwrap();
    let removed = contents.remove_one(&items).unwrap();
    assert!(removed.matches(&ItemStack::new(ore, 3)));
    let overridden = contents.remove_one(&items).unwrap();
    assert_eq!(overridden.max_stack_size(&items), 16);
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
