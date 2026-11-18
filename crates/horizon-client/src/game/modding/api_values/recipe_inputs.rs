//! Read-only recipe inputs: resident inventory limits apply to writes, not predicates.
#[cfg(test)]
use super::{ApiStack, StackWriteError};
pub(in crate::game) use horizon_gameplay::api_values::recipe_inputs::*;
#[cfg(test)]
use horizon_items::items::Items;
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recipe_reads_preserve_values_beyond_resident_limits_and_refuse_missing_items() {
        mod_platform::__private::part_tests::install_parts_fixture(&[]).unwrap();
        mod_api::__private::resolve_items([
            "minecraft:stick",
            "minecraft:iron_sword",
            "test:absent",
        ])
        .unwrap();
        let root =
            std::env::temp_dir().join(format!("horizon-recipe-inputs-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("items.json"),
            r#"{
            "minecraft:stick":{"minecraft:max_stack_size":64},
            "minecraft:iron_sword":{"minecraft:max_stack_size":1,"minecraft:max_damage":250}
        }"#,
        )
        .unwrap();
        let defaults = horizon_world::item_defaults::read(&root).unwrap();
        let items =
            Items::catalog(&root, &horizon_world::loot::PackOverlay::NONE, &defaults).unwrap();
        std::fs::remove_dir_all(root).unwrap();
        let key = |name| mod_api::__private::item_key(name).unwrap();
        let many = ApiStack::new(key("minecraft:stick"), 65).unwrap();
        let read = stack(&many, &items).unwrap();
        assert_eq!((&*read.item, read.count), ("minecraft:stick", 65));
        let mut damaged = ApiStack::new(key("minecraft:iron_sword"), 1).unwrap();
        damaged.set_damage(500);
        let read = stack(&damaged, &items).unwrap();
        assert_eq!(
            (&*read.item, read.count, read.damage),
            ("minecraft:iron_sword", 1, 500)
        );
        assert_eq!(
            stack(&ApiStack::of(key("test:absent")), &items),
            Err(StackWriteError::UnknownItem)
        );
    }
}
