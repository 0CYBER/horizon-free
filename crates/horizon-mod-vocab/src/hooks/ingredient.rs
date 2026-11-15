//! The `ingredient` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0226

pub trait Hooks {
    /// A recipe ingredient of a mod's own kind: the entry named a type in a
    /// namespace outside `minecraft:`, and the loader kept it for this call.
    /// The kind and the converted NBT compound the file wrote are here. JSON `true` and
    /// `false` become `Byte` 1 and 0 (a bool cannot be told from a number); an integer
    /// becomes the smallest of `Byte`, `Short`, `Int` and `Long` that holds it, and so does
    /// a decimal that is an exact integer fitting in `Long` (`1.0`, `1e3`, `-0.0`). Any other
    /// decimal becomes `Float` if it round-trips through `f32`, else `Double`. The width depends
    /// on the value, so read numeric tags by value across variants. Fill `items` with the
    /// item ids the ingredient matches (item ids only: components are not
    /// checked here; a match-time hook follows separately). A non-empty list rejects every
    /// item outside it before the match-time hook runs, so it must contain every item the
    /// hook should accept; the hook can only narrow it. Duplicates are
    /// removed keeping first-seen order. The hook is called once per
    /// occurrence of the kind (nested inside a compound, intersection or
    /// difference too, and under any slot a recipe reads an ingredient from) in
    /// each load, before the load's tables parse the recipes, with the
    /// `DataLoadContext` of that owning load: the client's own recipe load has
    /// `origin()` `ClientPreload`, the server's has `ServerDataLoad`. A server
    /// loads its recipes once, however many tables and copies read them (its
    /// recipe tables, the stations and the copy synchronised to guests, on a
    /// dedicated server too), and a `/reload` asks again once; a client that
    /// only joins a remote server is never told `ServerDataLoad`. The answer
    /// must be the same every time. With no listener at all, a recipe naming a custom kind
    /// is refused at load with an error naming the file and the kind. An empty list retains the custom
    /// predicate but makes the recipe unplaceable: the loader warns (naming the file and the kind), omits
    /// property sets and stonecutter selection, and refuses book placement.
    #[hook(
        side = "both",
        contract = "collect",
        out = "items",
        context = "data_load",
        version = 3,
        target = "horizon_client_lib::crafting::native_hooks::native_ingredient",
        neoforge = "ICustomIngredient"
    )]
    fn ingredient();
}
