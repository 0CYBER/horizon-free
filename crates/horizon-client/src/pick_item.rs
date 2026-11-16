//! Pick Block on the player's inventory: `ServerGamePacketListenerImpl.tryPickItem`
//! with `Inventory.findSlotMatchingItem`, `pickSlot`, `addAndPickItem` and
//! `getSuitableHotbarSlot`. `items` are the 36 main slots (0-8 the hotbar).
use crate::items::ItemStack;

/// Picks `stack` (the block's clone item): selects a matching stack already
/// in the hotbar, or swaps one from the main inventory into a suitable
/// hotbar slot; with infinite materials (creative) it adds the item when
/// none is there. Returns whether anything changed. Vanilla's rules: a pick
/// keeps an enchanted hotbar stack where it is.
pub fn pick(
    items: &mut [Option<ItemStack>],
    selected: &mut usize,
    stack: &ItemStack,
    infinite_materials: bool,
) -> bool {
    let picked = plan(items, *selected, stack, infinite_materials, &mut |s, _| {
        vanilla_not_replaceable(s)
    });
    picked.is_some_and(|picked| {
        apply(items, selected, stack, picked);
        true
    })
}

/// `ItemStack.isNotReplaceableByPickAction`'s vanilla answer: the stack is enchanted.
pub fn vanilla_not_replaceable(stack: &ItemStack) -> bool {
    !stack.enchantments().is_empty()
}

/// What a pick does to the inventory ([`plan`], then [`apply`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Picked {
    /// A matching hotbar stack: select its slot.
    Select(usize),
    /// `pickSlot`: select hotbar slot `hotbar` and swap main slot `from` into it.
    Swap { hotbar: usize, from: usize },
    /// `addAndPickItem`: select hotbar slot `hotbar` and put the item there.
    Add { hotbar: usize },
}

/// Decides a pick of `stack` without changing `items`: `None` when it does nothing.
/// `not_replaceable` is asked, with the hotbar stack and its slot, in
/// `getSuitableHotbarSlot`'s second pass, from the selected slot on until one answers
/// `false` (`IItemExtension.isNotReplaceableByPickAction`).
pub fn plan(
    items: &[Option<ItemStack>],
    selected: usize,
    stack: &ItemStack,
    infinite_materials: bool,
    not_replaceable: &mut dyn FnMut(&ItemStack, usize) -> bool,
) -> Option<Picked> {
    let existing = items.iter().position(|s| {
        s.as_ref()
            .is_some_and(|s| s.same_item_same_components(stack))
    });
    match existing {
        Some(slot) if slot < 9 => Some(Picked::Select(slot)),
        Some(from) => Some(Picked::Swap {
            hotbar: suitable_hotbar_slot(items, selected, not_replaceable),
            from,
        }),
        None if infinite_materials => Some(Picked::Add {
            hotbar: suitable_hotbar_slot(items, selected, not_replaceable),
        }),
        None => None,
    }
}

/// Carries out a pick [`plan`] decided for `stack`.
pub fn apply(
    items: &mut [Option<ItemStack>],
    selected: &mut usize,
    stack: &ItemStack,
    picked: Picked,
) {
    match picked {
        Picked::Select(slot) => *selected = slot,
        Picked::Swap { hotbar, from } => {
            // pickSlot.
            *selected = hotbar;
            items.swap(hotbar, from);
        }
        Picked::Add { hotbar } => {
            // addAndPickItem: what the hotbar slot held moves to a free slot
            // (or is replaced when there is none).
            *selected = hotbar;
            if items[hotbar].is_some()
                && let Some(free) = items.iter().position(Option::is_none)
            {
                items[free] = items[hotbar].take();
            }
            items[hotbar] = Some(stack.clone());
        }
    }
}

/// `getSuitableHotbarSlot`: from the selected slot on, the first empty one,
/// else the first `not_replaceable` answers `false` for, else the selected one.
fn suitable_hotbar_slot(
    items: &[Option<ItemStack>],
    selected: usize,
    not_replaceable: &mut dyn FnMut(&ItemStack, usize) -> bool,
) -> usize {
    let order = || (0..9).map(|i| (selected + i) % 9);
    order()
        .find(|&i| items[i].is_none())
        .or_else(|| order().find(|&i| items[i].as_ref().is_some_and(|s| !not_replaceable(s, i))))
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

    /// Enchanted diamond sword, stone, then dirt × 7 in the hotbar, oak planks in main slot 20.
    fn full_hotbar() -> Vec<Option<ItemStack>> {
        let mut items = inventory(&[(1, "stone"), (20, "oak_planks")]);
        let mut sword = stack("diamond_sword", 1).unwrap();
        sword.enchant("minecraft:sharpness", 1);
        items[0] = Some(sword);
        for slot in 2..9 {
            items[slot] = stack("dirt", 5);
        }
        items
    }

    #[test]
    fn the_rule_picks_the_hotbar_slot_a_full_hotbar_gives_up() {
        let planks = stack("oak_planks", 1).unwrap();
        let pick_with = |rule: &mut dyn FnMut(&ItemStack, usize) -> bool| {
            let mut items = full_hotbar();
            let mut selected = 0;
            let picked = plan(&items, selected, &planks, false, rule).unwrap();
            apply(&mut items, &mut selected, &planks, picked);
            (selected, items[20].as_ref().map(|s| s.item.to_string()))
        };
        // Vanilla: the enchanted sword stays, the stone goes to slot 20.
        let mut asked = Vec::new();
        let vanilla = pick_with(&mut |s, slot| {
            asked.push((s.item.to_string(), slot));
            vanilla_not_replaceable(s)
        });
        assert_eq!(vanilla, (1, Some("minecraft:stone".into())));
        // Asked from the selected slot on, until one answers false.
        assert_eq!(
            asked,
            [
                ("minecraft:diamond_sword".into(), 0),
                ("minecraft:stone".into(), 1)
            ]
        );
        let mut items = full_hotbar();
        let mut selected = 0;
        assert!(pick(&mut items, &mut selected, &planks, false));
        assert_eq!(selected, 1);
        // Stone kept in place: the dirt in slot 2 goes.
        let keep_stone = pick_with(&mut |s, slot| {
            vanilla_not_replaceable(s) || (&*s.item == "minecraft:stone" && slot == 1)
        });
        assert_eq!(keep_stone, (2, Some("minecraft:dirt".into())));
        // The enchanted sword let go: it goes to slot 20.
        let free_sword = pick_with(&mut |s, _| {
            &*s.item != "minecraft:diamond_sword" && vanilla_not_replaceable(s)
        });
        assert_eq!(free_sword, (0, Some("minecraft:diamond_sword".into())));
        // Every stack kept: the selected slot is used.
        assert_eq!(
            pick_with(&mut |_, _| true),
            (0, Some("minecraft:diamond_sword".into()))
        );
    }

    #[test]
    fn an_empty_slot_or_a_hotbar_match_never_asks() {
        let mut never = |_: &ItemStack, _: usize| -> bool { panic!("asked") };
        let items = inventory(&[(0, "dirt"), (20, "stone")]);
        assert_eq!(
            plan(&items, 0, &stone(), false, &mut never),
            Some(Picked::Swap {
                hotbar: 1,
                from: 20
            })
        );
        let items = inventory(&[(0, "dirt"), (4, "stone")]);
        assert_eq!(
            plan(&items, 0, &stone(), false, &mut never),
            Some(Picked::Select(4))
        );
    }
}
