//! Load and match hook adapters at the engine boundary.
use horizon_world::native_load_context::LoadContext;
use mod_api::RecipePropertyAddition;
/// A recipe ingredient of a mod's own kind: the entry named a type in a
/// namespace outside `minecraft:`, and the loader kept it for this call.
/// The kind and the converted NBT data the file wrote are here; fill `items` with the
/// item ids the ingredient matches (item ids only: components are not
/// checked here; a match-time hook follows separately). Duplicates are
/// removed keeping first-seen order. The load calls it once per occurrence
/// (nested ones included), before any table parses the recipes; the
/// answer must be the same every time. With no listener the load refuses the recipe file
/// before this runs; an empty answer keeps an unplaceable recipe.
#[inline(never)]
pub fn native_ingredient(
    context: &horizon_world::native_load_context::LoadContext,
    kind: &str,
    data: &horizon_core::nbt::Compound,
    items: &mut Vec<String>,
) {
    mod_api::dispatch_ingredient(
        &horizon_client_render::api_context::LoadReader(context).context(),
        kind,
        data,
        items,
    );
}
/// Items a mod's own recipe type takes in a station slot, added to the
/// recipe property sets (`RecipeManager.RECIPE_PROPERTY_SETS`): what a
/// furnace, blast furnace, smoker, campfire, smithing table slot or brewing
/// stand slot accepts, and what a client predicts it accepts. Push one
/// `RecipePropertyAddition` per set to add to; the vanilla recipes' own
/// items stay in. Called once per load, after the recipes are resolved, with
/// the `DataLoadContext` of the load that owns them (`origin()`
/// `ServerDataLoad` on a server, `ClientPreload` on a client's own load),
/// and again after `/reload`. New keys in manifest-owned namespaces create
/// custom input sets. Unknown vanilla keys, invalid or foreign new keys and
/// air are logged and skipped; unregistered items are ignored. The answer
/// must be the same every time.
#[inline(never)]
pub fn native_recipe_property_sets(
    context: &LoadContext,
    additions: &mut Vec<RecipePropertyAddition>,
) {
    mod_api::dispatch_recipe_property_sets(
        &horizon_client_render::api_context::LoadReader(context).context(),
        additions,
    );
}
/// A mod's own ingredient kind, matching a concrete stack at craft time:
/// the ingredient's `test` calls here with the kind, its NBT compound and the
/// stack being matched, `accept` already true (the load-time list named
/// the item). Set `accept` to false to refuse the stack — its components
/// are the usual reason. The hook may be called many times a craft; the
/// answer must be the same for the same stack.
#[inline(never)]
pub fn native_ingredient_test(
    context: &horizon_world::native_load_context::LoopContext,
    kind: &str,
    data: &horizon_core::nbt::Compound,
    stack: &mod_api::ItemStack,
    accept: &mut bool,
) {
    mod_api::dispatch_ingredient_test(
        &horizon_client_render::api_context::LoopReader(context).context(),
        kind,
        data,
        stack,
        accept,
    );
}
/// Whether two of `stack`'s item may be combined in a crafting grid to repair it
/// (`IItemExtension.isCombineRepairable`, which `RepairItemRecipe.canCombine` asks) is being
/// decided: `repairable` starts with vanilla's answer, whether the stack has durability, and
/// the parts of the stack's item may change it. Asked inside recipe matching on both sides.
#[inline(never)]
pub fn native_is_combine_repairable(
    context: &horizon_world::native_load_context::LoopContext,
    stack: &mod_api::ItemStack,
    repairable: &mut bool,
) {
    mod_api::dispatch_is_combine_repairable(
        &horizon_client_render::api_context::LoopReader(context).context(),
        stack,
        repairable,
    );
}
