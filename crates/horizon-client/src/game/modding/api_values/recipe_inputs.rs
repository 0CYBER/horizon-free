//! Read-only recipe inputs: resident inventory limits apply to writes, not predicates.
use super::{ApiStack, EngineStack, StackWriteError, encode_stack};
use horizon_items::items::Items;

/// Validate registry membership once while retaining the registry's shared item identity.
/// Counts and damage are API values, not attempted inventory writes.
pub(in crate::game) fn stack(
    value: &ApiStack,
    items: &Items,
) -> Result<EngineStack, StackWriteError> {
    let name = mod_api::__private::item_key_id(value.item()).ok_or(StackWriteError::UnknownItem)?;
    let (name, _) = items
        .items
        .get_key_value(name)
        .ok_or(StackWriteError::UnknownItem)?;
    encode_stack(value, name.clone())
}

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
