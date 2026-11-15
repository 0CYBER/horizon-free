//! `Item.canFitInsideContainerItems`: whether a stack may go inside an item that holds
//! items, a shulker box or a bundle. One rule and one entry point, which asks the parts of
//! the stack's item (`ItemPart::can_fit_inside_container_items`) after vanilla's answer.
use super::ItemStack;
use horizon_world::native_load_context::LoopContext;

/// Vanilla's `Item.canFitInsideContainerItems`: a shulker box can't go inside an item that
/// holds items (`ShulkerBoxBlock` items answer false).
pub fn vanilla_fits_inside_container_items(stack: &ItemStack) -> bool {
    !stack.item.ends_with("shulker_box")
}

/// `ItemStack.canFitInsideContainerItems` (`ShulkerBoxSlot.mayPlace`): vanilla's answer,
/// then the parts of the stack's item. An item with no such part keeps vanilla's answer at
/// the cost of one table lookup. `context` is the loop the menu or the insert runs in.
pub fn fits_inside_container_items(context: &LoopContext, stack: &ItemStack) -> bool {
    let mut fits = vanilla_fits_inside_container_items(stack);
    crate::native_client_hooks::can_fit_inside_container_items(context, stack, &mut fits);
    fits
}

/// `BundleContents.canItemBeInBundle`: a stack that isn't empty and fits inside container
/// items ([`fits_inside_container_items`]).
pub fn can_be_in_bundle(context: &LoopContext, stack: &ItemStack) -> bool {
    stack.count > 0 && fits_inside_container_items(context, stack)
}

/// Whether `stack` may go inside an item that holds items
/// (`IItemExtension.canFitInsideContainerItems`) is being decided: `fits` starts with
/// vanilla's answer and the parts of the stack's item may change it. Asked by a shulker
/// box's slots, a bundle and a mod's item slots, on both sides.
#[inline(never)]
pub fn native_can_fit_inside_container_items(
    context: &LoopContext,
    stack: &mod_api::ItemStack,
    fits: &mut bool,
) {
    mod_api::dispatch_can_fit_inside_container_items(
        &horizon_client_render::api_context::LoopReader(context).context(),
        stack,
        fits,
    );
}

#[cfg(test)]
#[path = "container_items_tests.rs"]
mod tests;
