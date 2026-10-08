//! Player inventory resource insertion. The source stack is consumed in place;
//! callbacks can mutate aliased stacks or inventory slots at native boundaries.
use crate::{
    item_stack_refs::StackRef,
    player_inventory::{InventoryError, InventoryRules, PlayerInventory},
};
#[derive(Debug)]
pub enum PickupError<E> {
    Inventory(InventoryError),
    Callback(E),
}
pub trait PickupHooks<I, K, V>: InventoryRules<I, K, V> {
    type Error;
    fn item_max_stack_size(&self, stack: &StackRef<I, K, V>) -> i32;
    fn max_stack_size(
        &mut self,
        inventory: &mut PlayerInventory<I, K, V>,
    ) -> Result<i32, Self::Error>;
    fn infinite_materials(
        &mut self,
        inventory: &mut PlayerInventory<I, K, V>,
    ) -> Result<bool, Self::Error>;
}
fn limit<I: Copy + Eq, K: Clone + Eq, V: Clone + PartialEq, H: PickupHooks<I, K, V>>(
    inv: &mut PlayerInventory<I, K, V>,
    stack: &StackRef<I, K, V>,
    hooks: &mut H,
) -> Result<i32, PickupError<H::Error>> {
    let max = hooks.max_stack_size(inv).map_err(PickupError::Callback)?;
    Ok(max.min(hooks.item_max_stack_size(stack)))
}
fn remaining<I: Copy + Eq, K: Clone + Eq, V: Clone + PartialEq, H: PickupHooks<I, K, V>>(
    inv: &mut PlayerInventory<I, K, V>,
    target: &StackRef<I, K, V>,
    stack: &StackRef<I, K, V>,
    hooks: &mut H,
) -> Result<bool, PickupError<H::Error>> {
    if target.read().is_empty()
        || !target.read().same_item_components(&stack.read())
        || hooks.item_max_stack_size(target) <= 1
        || hooks.is_damaged(target)
    {
        return Ok(false);
    }
    let count = target.read().count();
    Ok(count < limit(inv, target, hooks)?)
}
pub fn slot_with_space<
    I: Copy + Eq,
    K: Clone + Eq,
    V: Clone + PartialEq,
    H: PickupHooks<I, K, V>,
>(
    inv: &mut PlayerInventory<I, K, V>,
    stack: &StackRef<I, K, V>,
    hooks: &mut H,
) -> Result<i32, PickupError<H::Error>> {
    let target = inv.selected_item();
    if remaining(inv, &target, stack, hooks)? {
        return Ok(inv.selected() as i32);
    }
    let target = inv.get(40).expect("offhand");
    if remaining(inv, &target, stack, hooks)? {
        return Ok(40);
    }
    for i in 0..36 {
        let target = inv.get(i).expect("valid slot");
        if remaining(inv, &target, stack, hooks)? {
            return Ok(i);
        }
    }
    Ok(-1)
}
fn add_resource<I: Copy + Eq, K: Clone + Eq, V: Clone + PartialEq, H: PickupHooks<I, K, V>>(
    inv: &mut PlayerInventory<I, K, V>,
    index: i32,
    stack: &StackRef<I, K, V>,
    hooks: &mut H,
) -> Result<i32, PickupError<H::Error>> {
    let count = stack.read().count();
    let mut target = inv.get(index).map_err(PickupError::Inventory)?;
    if target.read().is_empty() {
        target = inv.stacks().resolve(stack.read().copy_with_count(0));
        inv.set(index, target.clone())
            .map_err(PickupError::Inventory)?;
    }
    let max = limit(inv, &target, hooks)?;
    let amount = count.min(max.wrapping_sub(target.read().count()));
    if amount == 0 {
        return Ok(count);
    }
    let count = count.wrapping_sub(amount);
    target.write().grow(amount);
    target.write().pop_time = 5;
    Ok(count)
}
pub fn add<I: Copy + Eq, K: Clone + Eq, V: Clone + PartialEq, H: PickupHooks<I, K, V>>(
    inv: &mut PlayerInventory<I, K, V>,
    mut index: i32,
    stack: &StackRef<I, K, V>,
    hooks: &mut H,
) -> Result<bool, PickupError<H::Error>> {
    if stack.read().is_empty() {
        return Ok(false);
    }
    if hooks.is_damaged(stack) {
        if index == -1 {
            index = inv.free_slot();
        }
        if index >= 0 {
            // copyAndClear is evaluated before List.set validates its index.
            let copy = inv.stacks().resolve(stack.write().copy_and_clear());
            inv.set_non_equipment(index, copy)
                .map_err(PickupError::Inventory)?;
            inv.get(index)
                .expect("validated ordinary slot")
                .write()
                .pop_time = 5;
            return Ok(true);
        }
        if hooks
            .infinite_materials(inv)
            .map_err(PickupError::Callback)?
        {
            stack.write().count = 0;
            return Ok(true);
        }
        return Ok(false);
    }
    let before = loop {
        let before = stack.read().count();
        let left = if index == -1 {
            let mut slot = slot_with_space(inv, stack, hooks)?;
            if slot == -1 {
                slot = inv.free_slot();
            }
            if slot == -1 {
                stack.read().count()
            } else {
                add_resource(inv, slot, stack, hooks)?
            }
        } else {
            add_resource(inv, index, stack, hooks)?
        };
        stack.write().count = left;
        if stack.read().is_empty() || stack.read().count() >= before {
            break before;
        }
    };
    if stack.read().count() == before
        && hooks
            .infinite_materials(inv)
            .map_err(PickupError::Callback)?
    {
        stack.write().count = 0;
        return Ok(true);
    }
    Ok(stack.read().count() < before)
}
