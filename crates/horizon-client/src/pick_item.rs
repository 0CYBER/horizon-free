//! Pick Block on the player's inventory: `ServerGamePacketListenerImpl.tryPickItem`
//! with `Inventory.findSlotMatchingItem`, `pickSlot`, `addAndPickItem` and
//! `getSuitableHotbarSlot`. `items` are the 36 main slots (0-8 the hotbar).
use crate::items::ItemStack;

/// Picks `stack` (the block's clone item): selects a matching stack already
/// in the hotbar, or swaps one from the main inventory into a suitable
/// hotbar slot; with infinite materials (creative) it adds the item when
/// none is there. Returns whether anything changed.
pub fn pick(
    items: &mut [Option<ItemStack>],
    selected: &mut usize,
    stack: &ItemStack,
    infinite_materials: bool,
) -> bool {
    let existing = items.iter().position(|s| {
        s.as_ref()
            .is_some_and(|s| s.same_item_same_components(stack))
    });
    match existing {
        Some(slot) if slot < 9 => *selected = slot,
        Some(slot) => {
            // pickSlot.
            *selected = suitable_hotbar_slot(items, *selected);
            items.swap(*selected, slot);
        }
        None if infinite_materials => {
            // addAndPickItem: what the hotbar slot held moves to a free slot
            // (or is replaced when there is none).
            *selected = suitable_hotbar_slot(items, *selected);
            if items[*selected].is_some()
                && let Some(free) = items.iter().position(Option::is_none)
            {
                items[free] = items[*selected].take();
            }
            items[*selected] = Some(stack.clone());
        }
        None => return false,
    }
    true
}

/// `getSuitableHotbarSlot`: from the selected slot on, the first empty one,
/// else the first without enchantments, else the selected one.
fn suitable_hotbar_slot(items: &[Option<ItemStack>], selected: usize) -> usize {
    let from = |ok: &dyn Fn(&Option<ItemStack>) -> bool| {
        (0..9).map(|i| (selected + i) % 9).find(|&i| ok(&items[i]))
    };
    from(&|s| s.is_none())
        .or_else(|| from(&|s| s.as_ref().is_some_and(|s| s.enchantments().is_empty())))
        .unwrap_or(selected)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stack(item: &str, count: u32) -> Option<ItemStack> {
        Some(ItemStack {
            item: format!("minecraft:{item}").into(),
            count,
            damage: 0,
            extra: None,
        })
    }
    fn inventory(filled: &[(usize, &str)]) -> Vec<Option<ItemStack>> {
        let mut items = vec![None; 36];
        for &(i, item) in filled {
            items[i] = stack(item, 5);
        }
        items
    }
    fn stone() -> ItemStack {
        stack("stone", 1).unwrap()
    }

    #[test]
    fn survival_selects_a_matching_hotbar_stack() {
        let mut items = inventory(&[(0, "dirt"), (6, "stone"), (20, "stone")]);
        let mut selected = 2;
        assert!(pick(&mut items, &mut selected, &stone(), false));
        assert_eq!(selected, 6);
        assert_eq!(
            items,
            inventory(&[(0, "dirt"), (6, "stone"), (20, "stone")])
        );
    }

    #[test]
    fn survival_swaps_a_main_inventory_stack_into_the_hotbar() {
        // The first empty hotbar slot from the selected one, wrapping round.
        let mut items = inventory(&[(3, "dirt"), (4, "dirt"), (20, "stone")]);
        let mut selected = 3;
        assert!(pick(&mut items, &mut selected, &stone(), false));
        assert_eq!(selected, 5);
        assert_eq!(items[5], stack("stone", 5));
        assert_eq!(items[20], None);
        // A full hotbar: the selected slot (nothing is enchanted) trades places.
        let mut items: Vec<_> = (0..9).map(|_| stack("dirt", 5)).collect();
        items.extend((9..36).map(|i| if i == 30 { stack("stone", 5) } else { None }));
        let mut selected = 7;
        assert!(pick(&mut items, &mut selected, &stone(), false));
        assert_eq!(selected, 7);
        assert_eq!(
            (items[7].clone(), items[30].clone()),
            (stack("stone", 5), stack("dirt", 5))
        );
    }

    #[test]
    fn survival_without_the_item_does_nothing_and_creative_adds_it() {
        let before = inventory(&[(0, "dirt")]);
        let mut items = before.clone();
        let mut selected = 0;
        assert!(!pick(&mut items, &mut selected, &stone(), false));
        assert_eq!((items, selected), (before.clone(), 0));
        // Creative: the next empty hotbar slot takes one.
        let mut items = before;
        assert!(pick(&mut items, &mut selected, &stone(), true));
        assert_eq!(selected, 1);
        assert_eq!(items[1], stack("stone", 1));
        // A full hotbar: the slot's stack moves to the first free slot.
        let mut items: Vec<_> = (0..9).map(|_| stack("dirt", 5)).collect();
        items.extend(vec![None; 27]);
        let mut selected = 4;
        assert!(pick(&mut items, &mut selected, &stone(), true));
        assert_eq!(
            (selected, items[4].clone(), items[9].clone()),
            (4, stack("stone", 1), stack("dirt", 5))
        );
    }
}
