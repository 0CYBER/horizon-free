use super::*;

/// Whether the feet slot supports walking on powder snow.
pub(crate) fn leather_boots(armor: &[Option<ItemStack>; 4]) -> bool {
    armor[3]
        .as_ref()
        .is_some_and(|s| s.count > 0 && s.item.as_ref() == "minecraft:leather_boots")
}
