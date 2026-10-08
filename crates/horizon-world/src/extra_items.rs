//! Server-player delivery of extra items created by consuming an item.
use crate::{
    inventory_pickup::{self, PickupError, PickupHooks},
    item_stack_refs::StackRef,
    player_inventory::PlayerInventory,
};
pub trait ExtraItemHooks<I, K, V>: PickupHooks<I, K, V> {
    /// Native drop uses throwRandomly=false and Prediction::PREDICTED.
    fn drop_predicted(
        &mut self,
        inventory: &mut PlayerInventory<I, K, V>,
        stack: StackRef<I, K, V>,
    ) -> Result<(), Self::Error>;
}
pub fn deliver<I: Copy + Eq, K: Clone + Eq, V: Clone + PartialEq, H: ExtraItemHooks<I, K, V>>(
    inventory: &mut PlayerInventory<I, K, V>,
    stack: StackRef<I, K, V>,
    hooks: &mut H,
) -> Result<(), PickupError<H::Error>> {
    if !inventory_pickup::add(inventory, -1, &stack, hooks)? {
        hooks
            .drop_predicted(inventory, stack)
            .map_err(PickupError::Callback)?;
    }
    Ok(())
}
