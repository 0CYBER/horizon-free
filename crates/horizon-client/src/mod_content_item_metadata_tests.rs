//! Item metadata assertions for the prepared mod-content fixture.
use super::*;

pub(super) fn assert_metadata(
    root: &Path,
    items: &Items,
    defaults: &horizon_world::item_defaults::ItemDefaults,
    ruby: BlockStateId,
    n: u32,
) {
    let ruby_block = &items.items["ruby:ruby_block"];
    assert_eq!(ruby_block.block, Some(ruby));
    let iron_block = &items.items["minecraft:iron_block"];
    assert_eq!((ruby_block.max_stack, iron_block.max_stack), (16, 64));
    let damage = crate::health::HealthResources::load(root).unwrap();
    for (name, expected) in [("ruby:ruby_block", false), ("minecraft:iron_block", true)] {
        let stack = crate::items::ItemStack::new(name, 1);
        let default = defaults.entries()[name].get("minecraft:damage_resistant");
        assert_eq!(
            crate::item_damage::stack_can_be_hurt_by(
                default,
                &damage.damage,
                &stack,
                "minecraft:lava"
            ),
            Some(expected)
        );
    }
    let gem = &items.items["ruby:ruby"];
    assert_eq!(gem.max_stack, 16);
    assert_eq!(gem.block, Some(BlockStateId(n + 65)));
}
