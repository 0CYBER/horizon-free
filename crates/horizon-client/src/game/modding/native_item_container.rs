//! Items held by item stacks for native mods (docs/modding/src/how-to/native-mods.md): a mod's
//! backpacks, storage cells and toolboxes (`native_item_container`), read,
//! filled and emptied. NeoForge's `Capabilities.Item.ITEM` is a
//! `ResourceHandler<ItemResource>` reached through an `ItemAccess`: it acts on
//! one item of a stack at a time and may change it. So do these, as the item
//! fluid and energy helpers do: they take a stack and answer without changing
//! it, and the caller shrinks its stack by one and hands out the changed item
//! they return. The contents live in `minecraft:container`, as a shulker box
//! item keeps them. They hold no world state, so they work on any side.
use super::Game;
use horizon_client::items::ItemStack;
use horizon_core::nbt::Tag;

/// `ItemContainerContents`' limit on slots.
const MAX_SLOTS: u32 = 256;
/// `Container.getMaxStackSize`.
const CONTAINER_MAX: u32 = 99;

/// The slots of one item of `stack`, empty ones included; slots past `size`
/// are dropped, as the item has no room for them.
fn contents(stack: &ItemStack, size: u32) -> Vec<Option<ItemStack>> {
    let mut slots = vec![None; size as usize];
    for (slot, item) in stack.data().map_or(&[][..], |d| &d.container[..]) {
        if let Some(s) = slots.get_mut(usize::from(*slot))
            && item.count > 0
        {
            *s = Some(item.clone());
        }
    }
    slots
}

/// One item of `stack`, unchanged.
fn one_of(stack: &ItemStack) -> ItemStack {
    let mut one = stack.clone();
    one.count = 1;
    one
}

/// One item of `stack` holding `slots`, its other data kept. An empty item
/// carries nothing, so it stacks with a new one.
fn with_contents(stack: &ItemStack, slots: &[Option<ItemStack>]) -> ItemStack {
    let mut one = one_of(stack);
    let data = one.data_mut();
    data.container = slots
        .iter()
        .enumerate()
        .filter_map(|(i, s)| Some((i as u8, s.clone().filter(|s| s.count > 0)?)))
        .collect();
    // What a server sent for the stack no longer describes it; kept, an
    // emptied item would not stack with a new one.
    data.wire = Default::default();
    one.tidy();
    one
}

/// `SimpleContainer.addItem` on one item of `stack`: `item` tops up the
/// slots that hold the same item and components, then goes into the first
/// empty ones. Unlike `SimpleContainer`, which puts the whole rest in the
/// first empty slot and cuts it to the limit, no slot takes more than its
/// limit (`limit`), and what doesn't fit stays out. Nothing goes in when
/// `fits` is false (`ItemStack.canFitInsideContainerItems`, which the caller
/// asks). The count that went in, and the item it becomes.
fn insert_one(
    stack: &ItemStack,
    size: Option<u32>,
    item: &ItemStack,
    limit: u32,
    fits: bool,
) -> Option<(u32, ItemStack)> {
    let size = size?;
    if stack.count == 0 {
        return None;
    }
    if item.count == 0 || !fits {
        return Some((0, one_of(stack)));
    }
    let mut slots = contents(stack, size);
    let mut left = item.count;
    for held in slots.iter_mut().flatten() {
        if left == 0 {
            break;
        }
        if held.same_item_same_components(item) {
            let room = limit.saturating_sub(held.count).min(left);
            held.count += room;
            left -= room;
        }
    }
    for slot in slots.iter_mut().filter(|s| s.is_none()) {
        if left == 0 {
            break;
        }
        let put = limit.min(left);
        *slot = Some(ItemStack {
            count: put,
            ..item.clone()
        });
        left -= put;
    }
    let moved = item.count - left;
    if moved == 0 {
        return Some((0, one_of(stack)));
    }
    Some((moved, with_contents(stack, &slots)))
}

/// As much of `item` as slot `slot` of one item of `stack` takes, the slot topped up when it
/// holds the same item and components, filled when empty, never past `limit`; none into a slot
/// past the item's slots, or holding another item, nor when `fits` is false. The count that
/// went in, and the item it becomes.
fn insert_into(
    stack: &ItemStack,
    size: Option<u32>,
    slot: usize,
    item: &ItemStack,
    limit: u32,
    fits: bool,
) -> Option<(u32, ItemStack)> {
    let size = size?;
    if stack.count == 0 {
        return None;
    }
    let mut slots = contents(stack, size);
    let room = match slots.get(slot) {
        _ if !fits => 0,
        Some(None) => limit,
        Some(Some(held)) if held.same_item_same_components(item) => {
            limit.saturating_sub(held.count)
        }
        _ => 0,
    };
    let put = room.min(item.count);
    if put == 0 {
        return Some((0, one_of(stack)));
    }
    let held = slots[slot].get_or_insert_with(|| ItemStack {
        count: 0,
        ..item.clone()
    });
    held.count += put;
    Some((put, with_contents(stack, &slots)))
}

/// Up to `max` out of `slot` of one item of `stack`: what came out (none
/// when nothing did), and the item it becomes.
fn extract_one(
    stack: &ItemStack,
    size: Option<u32>,
    slot: usize,
    max: u32,
) -> Option<(Option<ItemStack>, ItemStack)> {
    let size = size?;
    if stack.count == 0 {
        return None;
    }
    let mut slots = contents(stack, size);
    let Some(Some(held)) = slots.get_mut(slot) else {
        return Some((None, one_of(stack)));
    };
    let take = held.count.min(max);
    if take == 0 {
        return Some((None, one_of(stack)));
    }
    let taken = ItemStack {
        count: take,
        ..held.clone()
    };
    held.count -= take;
    Some((Some(taken), with_contents(stack, &slots)))
}

// Native mods use these; nothing in Horizon does.
impl Game<'_> {
    /// `ItemStack.canFitInsideContainerItems`: vanilla's answer, then the parts of `item`'s
    /// item, in the loop this game runs.
    fn fits_inside_container_items(&self, item: &ItemStack) -> bool {
        horizon_client::menu::fits_inside_container_items(&self.loop_context, item)
    }
    /// The slots a mod gave item `item`, clamped; none for any other item.
    fn item_container_size(&self, item: &str) -> Option<u32> {
        let mut slots = None;
        self.native_item_container(item, &mut slots);
        slots.filter(|s| *s >= 1).map(|s| s.min(MAX_SLOTS))
    }
    /// `Container.getMaxStackSize(stack)`: the container's 99, or the stack's
    /// own limit (its `max_stack_size` component, else the item's) when less.
    fn item_container_limit(&self, stack: &ItemStack) -> u32 {
        let own = match stack
            .data()
            .and_then(|d| d.other("minecraft:max_stack_size"))
        {
            Some(Tag::Int(n)) => (*n).clamp(1, 99) as u32,
            _ => self
                .res
                .items()
                .get(&stack.item)
                .map_or(64, |i| i.max_stack),
        };
        own.min(CONTAINER_MAX)
    }
    /// `Capabilities.Item.ITEM`, read: the slots of one item of `stack`,
    /// empty ones included, kept in its `minecraft:container`. None for an
    /// item without slots, and for an empty stack.
    pub(in crate::game) fn item_container_slots(
        &self,
        stack: &ItemStack,
    ) -> Option<Vec<Option<ItemStack>>> {
        let size = self.item_container_size(&stack.item)?;
        (stack.count > 0).then(|| contents(stack, size))
    }
    /// `ResourceHandler.insert`: as much of `item` as fits into one item of
    /// `stack`, topping up the slots that hold the same item and components
    /// first, then the first empty ones (`SimpleContainer.addItem`'s order),
    /// no slot past its limit (99, or the item's own when less). A shulker
    /// box goes in only when its parts say it fits (`can_fit_inside_container_items`).
    /// The count that went in, and the single item it
    /// becomes (the item itself when none did). The stack given is not
    /// changed: shrink it by one and hand out the result, or leave the result
    /// unused to simulate. None where the item has no slots.
    pub(in crate::game) fn item_container_insert(
        &self,
        stack: &ItemStack,
        item: &ItemStack,
    ) -> Option<(u32, ItemStack)> {
        let size = self.item_container_size(&stack.item);
        let fits = self.fits_inside_container_items(item);
        insert_one(stack, size, item, self.item_container_limit(item), fits)
    }
    /// `ResourceHandler.insert(index, …)`: as `item_container_insert`,
    /// into slot `slot` alone: topped up when it holds the same item and
    /// components, filled when empty. Nothing goes into a slot past the
    /// item's slots or holding another item.
    pub(in crate::game) fn item_container_insert_slot(
        &self,
        stack: &ItemStack,
        slot: usize,
        item: &ItemStack,
    ) -> Option<(u32, ItemStack)> {
        let size = self.item_container_size(&stack.item);
        let fits = self.fits_inside_container_items(item);
        insert_into(
            stack,
            size,
            slot,
            item,
            self.item_container_limit(item),
            fits,
        )
    }
    /// `ResourceHandler.isValid` and `getCapacityAsLong`: how many of `item`
    /// slot `slot` of one item of `stack` holds when empty. None of a stack that
    /// doesn't fit inside container items (a shulker box, unless its parts say
    /// so), nor into a slot past the item's slots.
    pub(in crate::game) fn item_container_capacity(
        &self,
        stack: &ItemStack,
        slot: usize,
        item: &ItemStack,
    ) -> u32 {
        let fits = self
            .item_container_size(&stack.item)
            .is_some_and(|size| slot < size as usize)
            && self.fits_inside_container_items(item);
        if fits {
            self.item_container_limit(item)
        } else {
            0
        }
    }
    /// `ResourceHandler.extract`: up to `max` out of slot `slot` of one item
    /// of `stack`. What came out (none when the slot is empty or past the
    /// item's slots), and the single item it becomes. The stack given is not
    /// changed, as with `item_container_insert`. None where the item
    /// has no slots.
    pub(in crate::game) fn item_container_extract(
        &self,
        stack: &ItemStack,
        slot: usize,
        max: u32,
    ) -> Option<(Option<ItemStack>, ItemStack)> {
        let size = self.item_container_size(&stack.item);
        extract_one(stack, size, slot, max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_core::nbt::JavaString;
    use std::sync::Arc;

    fn bag(count: u32) -> ItemStack {
        ItemStack::new("mymod:bag", count)
    }

    fn of(item: &str, count: u32) -> ItemStack {
        ItemStack::new(item, count)
    }

    /// `super::insert_one` with vanilla's container rule, which no part changes here.
    fn insert_one(
        stack: &ItemStack,
        size: Option<u32>,
        item: &ItemStack,
        limit: u32,
    ) -> Option<(u32, ItemStack)> {
        let fits = horizon_client::menu::container_items::vanilla_fits_inside_container_items(item);
        super::insert_one(stack, size, item, limit, fits)
    }

    /// `super::insert_into` with vanilla's container rule.
    fn insert_into(
        stack: &ItemStack,
        size: Option<u32>,
        slot: usize,
        item: &ItemStack,
        limit: u32,
    ) -> Option<(u32, ItemStack)> {
        let fits = horizon_client::menu::container_items::vanilla_fits_inside_container_items(item);
        super::insert_into(stack, size, slot, item, limit, fits)
    }

    /// (slot, item, count) of what one item holds.
    fn held(stack: &ItemStack, size: u32) -> Vec<(usize, String, u32)> {
        contents(stack, size)
            .into_iter()
            .enumerate()
            .filter_map(|(i, s)| s.map(|s| (i, s.item.to_string(), s.count)))
            .collect()
    }

    #[test]
    fn an_insert_tops_up_equal_stacks_before_empty_slots() {
        let (_, one) = insert_one(&bag(1), Some(3), &of("minecraft:stone", 10), 64).unwrap();
        let (_, one) = insert_one(&one, Some(3), &of("minecraft:dirt", 5), 64).unwrap();
        let (moved, one) = insert_one(&one, Some(3), &of("minecraft:stone", 60), 64).unwrap();
        // 54 top up slot 0 to 64, the other 6 take the first empty slot.
        assert_eq!(moved, 60);
        assert_eq!(
            held(&one, 3),
            [
                (0, "minecraft:stone".into(), 64),
                (1, "minecraft:dirt".into(), 5),
                (2, "minecraft:stone".into(), 6),
            ]
        );
    }

    #[test]
    fn an_insert_into_a_slot_goes_there_alone() {
        let (moved, one) =
            insert_into(&bag(1), Some(2), 1, &of("minecraft:stone", 70), 64).unwrap();
        // Slot 1, not the first empty one, and only up to its limit.
        assert_eq!(moved, 64);
        assert_eq!(held(&one, 2), [(1, "minecraft:stone".into(), 64)]);
        let (moved, one) = insert_into(&one, Some(2), 1, &of("minecraft:stone", 10), 64).unwrap();
        assert_eq!((moved, held(&one, 2).len()), (0, 1));
        // Another item, or a slot past the item's, takes nothing.
        for slot in [1, 2] {
            let (moved, _) =
                insert_into(&one, Some(2), slot, &of("minecraft:dirt", 1), 64).unwrap();
            assert_eq!(moved, 0);
        }
        let (moved, one) = insert_into(&one, Some(2), 0, &of("minecraft:dirt", 1), 64).unwrap();
        assert_eq!(moved, 1);
        assert_eq!(
            held(&one, 2),
            [
                (0, "minecraft:dirt".into(), 1),
                (1, "minecraft:stone".into(), 64)
            ]
        );
    }

    #[test]
    fn no_slot_passes_its_limit_and_the_rest_stays_out() {
        // 16 per slot (ender pearls), two slots: 32 of 40 go in.
        let (moved, one) =
            insert_one(&bag(1), Some(2), &of("minecraft:ender_pearl", 40), 16).unwrap();
        assert_eq!(moved, 32);
        assert_eq!(
            held(&one, 2),
            [
                (0, "minecraft:ender_pearl".into(), 16),
                (1, "minecraft:ender_pearl".into(), 16),
            ]
        );
        // Full: nothing goes in, and the item comes back as it was.
        assert_eq!(
            insert_one(&one, Some(2), &of("minecraft:ender_pearl", 1), 16).unwrap(),
            (0, one.clone())
        );
    }

    #[test]
    fn different_components_never_share_a_slot() {
        let mut named = of("minecraft:stone", 1);
        named.data_mut().set_other(
            "minecraft:custom_name",
            Some(Tag::String(JavaString::from("Rock"))),
        );
        let (_, one) = insert_one(&bag(1), Some(2), &of("minecraft:stone", 1), 64).unwrap();
        let (moved, one) = insert_one(&one, Some(2), &named, 64).unwrap();
        assert_eq!(moved, 1);
        assert_eq!(held(&one, 2).len(), 2);
    }

    #[test]
    fn a_shulker_box_never_goes_in() {
        let shulker = of("minecraft:red_shulker_box", 1);
        assert_eq!(
            insert_one(&bag(1), Some(9), &shulker, 1).unwrap(),
            (0, bag(1))
        );
        // Another container item does.
        assert_eq!(insert_one(&bag(1), Some(9), &bag(1), 64).unwrap().0, 1);
    }

    #[test]
    fn an_extract_takes_up_to_max_and_an_emptied_bag_is_new_again() {
        let (_, full) = insert_one(&bag(1), Some(4), &of("minecraft:apple", 20), 64).unwrap();
        let (taken, rest) = extract_one(&full, Some(4), 0, 15).unwrap();
        assert_eq!(taken.map(|t| t.count), Some(15));
        assert_eq!(held(&rest, 4), [(0, "minecraft:apple".into(), 5)]);
        let (taken, empty) = extract_one(&rest, Some(4), 0, 64).unwrap();
        assert_eq!(
            taken.map(|t| (t.item.to_string(), t.count)),
            Some(("minecraft:apple".into(), 5))
        );
        assert_eq!(empty, bag(1));
        // An empty slot, or one past the bag's slots, gives nothing.
        assert_eq!(extract_one(&empty, Some(4), 0, 1).unwrap(), (None, bag(1)));
        assert_eq!(
            extract_one(&full, Some(4), 7, 1).unwrap(),
            (None, full.clone())
        );
    }

    #[test]
    fn the_stack_given_stays_and_one_item_comes_back() {
        let stack = bag(3);
        let (moved, one) = insert_one(&stack, Some(1), &of("minecraft:stone", 1), 64).unwrap();
        assert_eq!((moved, one.count, stack.count), (1, 1, 3));
        assert_eq!(stack, bag(3));
    }

    #[test]
    fn items_without_slots_hold_nothing() {
        assert_eq!(
            insert_one(&bag(1), None, &of("minecraft:stone", 1), 64),
            None
        );
        assert_eq!(extract_one(&bag(1), None, 0, 1), None);
        assert_eq!(
            insert_one(&bag(0), Some(3), &of("minecraft:stone", 1), 64),
            None
        );
        // Nothing to insert moves nothing.
        assert_eq!(
            insert_one(&bag(1), Some(3), &of("minecraft:stone", 0), 64).unwrap(),
            (0, bag(1))
        );
    }

    #[test]
    fn slots_past_the_size_are_dropped() {
        // A mod that shrank its bag after it was filled.
        let (_, big) = insert_one(&bag(1), Some(3), &of("minecraft:stone", 64 * 3), 64).unwrap();
        assert_eq!(held(&big, 2).len(), 2);
        assert_eq!(extract_one(&big, Some(2), 2, 64).unwrap().0, None);
    }

    #[test]
    fn an_emptied_bag_from_a_server_stacks_with_a_new_one() {
        let mut sent = bag(2);
        sent.data_mut().wire = horizon_client::item_data::Wire::new(bag(2), Arc::from(vec![1u8]));
        let (_, filled) = insert_one(&sent, Some(1), &of("minecraft:stone", 3), 64).unwrap();
        let (_, emptied) = extract_one(&filled, Some(1), 0, 3).unwrap();
        assert!(emptied.same_item_same_components(&bag(1)));
    }

    #[test]
    fn a_bag_keeps_the_rest_of_its_data() {
        let mut stack = bag(1);
        stack.data_mut().set_other(
            "minecraft:custom_name",
            Some(Tag::String(JavaString::from("Kit"))),
        );
        let (_, filled) = insert_one(&stack, Some(2), &of("minecraft:stone", 2), 64).unwrap();
        assert!(
            filled
                .data()
                .and_then(|d| d.other("minecraft:custom_name"))
                .is_some()
        );
        assert_eq!(held(&filled, 2), [(0, "minecraft:stone".into(), 2)]);
        let (_, emptied) = extract_one(&filled, Some(2), 0, 2).unwrap();
        assert_eq!(emptied, stack);
    }
}
