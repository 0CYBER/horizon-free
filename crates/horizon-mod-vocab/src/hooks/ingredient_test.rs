//! The `ingredient_test` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A mod's own ingredient kind, matching a concrete stack at craft time:
    /// the ingredient's `test` calls here with the kind, its NBT compound and the
    /// stack being matched, `accept` already true (a nonempty load-time list
    /// named the item; an empty custom list does not filter it). Set `accept` to false to refuse the stack — its components
    /// are the usual reason. The hook may be called many times a craft; the
    /// answer must be the same for the same stack. It is given the loop `context` of the side that runs the match (its registries, settings and mod list); it fires inside recipe matching, so it takes no `Host`.
    #[hook(
        side = "both",
        contract = "listen",
        context = "loop",
        version = 2,
        target = "horizon_client_lib::crafting::native_hooks::native_ingredient_test",
        neoforge = "ICustomIngredient"
    )]
    fn ingredient_test();
}
