//! The `recipe_property_sets` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Items a mod's own recipe type takes in a station slot, added to the
    /// recipe property sets (`RecipeManager.RECIPE_PROPERTY_SETS`): what a
    /// furnace, blast furnace, smoker, campfire, smithing table slot or brewing
    /// stand slot accepts, and what a client predicts it accepts. Push one
    /// `RecipePropertyAddition` per set to add to; the vanilla recipes' own
    /// items stay in. Called once per load, after the recipes are resolved, with
    /// the `DataLoadContext` of the load that owns them (`origin()`
    /// `ServerDataLoad` on a server, `ClientPreload` on a client's own load),
    /// and again after `/reload`. A new namespaced key creates a set when its
    /// namespace belongs to the listener's manifest (its ID or `namespaces`).
    /// Later listeners may extend existing sets. Invalid or foreign new keys,
    /// unknown vanilla keys, and air are logged and skipped; unknown items are
    /// ignored. Custom sets are synced to clients alongside vanilla sets. The
    /// answer must be the same every time.
    #[hook(
        side = "both",
        contract = "collect",
        out = "additions",
        collect_validator = "crate::item::crafting::validate_recipe_properties",
        context = "data_load",
        target = "horizon_client_lib::crafting::native_hooks::native_recipe_property_sets",
        neoforge = "RegisterRecipePropertiesEvent"
    )]
    fn recipe_property_sets();
}
