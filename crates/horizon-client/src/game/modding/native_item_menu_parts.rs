// game-module-visibility: pub(crate)
//! The executable's side of the item hooks the client library asks inside its menus and
//! recipe matching (`native_client_hooks`): `can_grindstone_repair`, `is_combine_repairable`
//! and `can_fit_inside_container_items`. Each asks the parts of the stack's item, with the
//! loop context of the side that runs the menu or the match, and no `Host`.
use super::api_values::TryIntoApi;
use horizon_client::items::ItemStack;
use horizon_world::native_load_context::LoopContext;

/// Says once per hook and item that a stack couldn't cross to the mods, so its parts weren't
/// asked and the answer stayed vanilla's.
fn refuse_stack(hook: &str, item: &str, error: super::api_values::StackReadError) {
    static REFUSED: std::sync::Mutex<std::collections::BTreeSet<(String, String)>> =
        std::sync::Mutex::new(std::collections::BTreeSet::new());
    if REFUSED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert((hook.to_owned(), item.to_owned()))
    {
        log::warn!(
            "Horizon: {hook} skipped for {item}: the stack can't be given to mods ({error:?}); vanilla's answer is kept"
        );
    }
}

/// Ask the parts of `stack`'s item whether the grindstone repairs it. With no part on the item,
/// nothing is converted and `can_repair` keeps its answer.
pub(crate) fn can_grindstone_repair(
    context: &LoopContext,
    stack: &ItemStack,
    can_repair: &mut bool,
) {
    if !mod_api::__private::item_listens(mod_api::HookId::CanGrindstoneRepair, &stack.item) {
        return;
    }
    match stack.try_into_api() {
        Ok(api) => {
            horizon_client::stations::native_can_grindstone_repair(context, &api, can_repair);
        }
        Err(error) => refuse_stack("can_grindstone_repair", &stack.item, error),
    }
}

/// Ask the parts of `stack`'s item whether two of it combine in a crafting grid. With no part on
/// the item, nothing is converted and `repairable` keeps its answer.
pub(crate) fn is_combine_repairable(
    context: &LoopContext,
    stack: &ItemStack,
    repairable: &mut bool,
) {
    if !mod_api::__private::item_listens(mod_api::HookId::IsCombineRepairable, &stack.item) {
        return;
    }
    match stack.try_into_api() {
        Ok(api) => horizon_client::crafting::native_hooks::native_is_combine_repairable(
            context, &api, repairable,
        ),
        Err(error) => refuse_stack("is_combine_repairable", &stack.item, error),
    }
}

/// Ask the parts of `stack`'s item whether it may go inside an item that holds items. With no
/// part on the item, nothing is converted and `fits` keeps its answer.
pub(crate) fn can_fit_inside_container_items(
    context: &LoopContext,
    stack: &ItemStack,
    fits: &mut bool,
) {
    let hook = mod_api::HookId::CanFitInsideContainerItems;
    if !mod_api::__private::item_listens(hook, &stack.item) {
        return;
    }
    match stack.try_into_api() {
        Ok(api) => horizon_client::menu::container_items::native_can_fit_inside_container_items(
            context, &api, fits,
        ),
        Err(error) => refuse_stack("can_fit_inside_container_items", &stack.item, error),
    }
}
