//! `Item.inventoryTick` for the stacks an entity carries: every living entity's
//! `EntityEquipment.tick` (`LivingEntity.aiStep`) and a player's `Inventory.tick`.
//! In 26.3 only `CompassItem` and `MapItem` override it, and `MapItem.update`
//! draws for players only (`game/blocks/maps`), so the compass is the rule here.
use horizon_client::{blocks::BlockFacts, items::ItemStack, world::World};
use horizon_core::nbt::Tag;
use horizon_items::equipment::Equipment;

/// `CompassItem.inventoryTick` -> `LodestoneTracker.tick`: a compass tracking
/// a lodestone in `dim` (the entity's level) that is gone, or out of the
/// world, gets `new LodestoneTracker(Optional.empty(), true)`. A target in an
/// unloaded chunk waits. A stack without `minecraft:lodestone_tracker`
/// returns at once, with nothing allocated.
pub(super) fn compass_inventory_tick(
    world: &World,
    facts: &BlockFacts,
    dim: &str,
    stack: &mut ItemStack,
) {
    if stack.item.as_ref() == "minecraft:compass" && lodestone_target_gone(world, facts, dim, stack)
    {
        stack.data_mut().set_other(
            "minecraft:lodestone_tracker",
            Some(Tag::Compound(Default::default())),
        );
    }
}

/// `EntityEquipment.tick`: `compass_inventory_tick` on each worn or held
/// stack, in `EquipmentSlot` order.
pub(super) fn equipment_inventory_tick(
    world: &World,
    facts: &BlockFacts,
    dim: &str,
    equipment: &mut Equipment,
) {
    for slot in 0..horizon_items::equipment::SLOT_NAMES.len() {
        if let Some(stack) = &mut equipment[slot] {
            compass_inventory_tick(world, facts, dim, stack);
        }
    }
}

/// `LodestoneTracker.tick`'s test: a tracked target in `dim` whose block,
/// in a loaded chunk, is no longer a lodestone.
fn lodestone_target_gone(world: &World, facts: &BlockFacts, dim: &str, stack: &ItemStack) -> bool {
    let Some(tracker) = stack
        .data()
        .and_then(|d| d.other("minecraft:lodestone_tracker"))
    else {
        return false;
    };
    let (Some(target), true) = (
        tracker.get("target"),
        horizon_client::save::number(tracker.get("tracked")).is_none_or(|v| v != 0),
    ) else {
        return false;
    };
    // `BlockPos.CODEC` reads exactly three ints; anything else is no target.
    let Some(&[x, y, z]) = (match target.get("pos") {
        Some(Tag::IntArray(pos)) => Some(pos.as_slice()),
        _ => None,
    }) else {
        return false;
    };
    matches!(target.get("dimension"), Some(Tag::String(name)) if name.0.iter().copied().eq(dim.encode_utf16()))
        && world.chunks.contains_key(&(x >> 4, z >> 4))
        && facts.get(world.block(x, y, z)).name.as_ref() != "minecraft:lodestone"
}
