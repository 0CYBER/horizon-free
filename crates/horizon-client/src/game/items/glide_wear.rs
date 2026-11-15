use super::*;

/// Equipment accepted by `LivingEntity.canGlideUsing`, including both hands.
/// Armor indices precede main hand (4) and off hand (5).
/// An iterator, so the per-tick glide checks allocate nothing.
pub(super) fn gliding_slots<'a>(
    items: &'a Items,
    armor: &'a [Option<ItemStack>; 4],
    mainhand: Option<&'a ItemStack>,
    offhand: Option<&'a ItemStack>,
) -> impl Iterator<Item = usize> + 'a {
    armor
        .iter()
        .map(Option::as_ref)
        .chain([mainhand, offhand])
        .zip(["head", "chest", "legs", "feet", "mainhand", "offhand"])
        .enumerate()
        .filter_map(move |(i, (stack, slot))| {
            stack
                .filter(|s| horizon_client::item_sync::can_glide_using(s, slot, items))
                .map(|_| i)
        })
}
