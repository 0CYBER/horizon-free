//! The `data_pack_files` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0110

pub trait Hooks {
    /// The server is assembling its data tables from the world's data packs
    /// (`ReloadableServerResources`): a world is opening, or `/reload` was
    /// run, and the recipes, tags, loot and advancements are built after
    /// this. Push `(path, bytes)` pack files and they join this load. `path`
    /// is the overlay's key, `<kind>/<namespace>:<path>` without `.json`, over
    /// the kinds the tables read: `recipe/mymod:thing`,
    /// `tags/mymod:block/thing`, `loot_table/…`, `advancement/…`,
    /// `trade_set/…`, `villager_trade/…`, `dialog/…`. A file that isn't JSON
    /// is skipped with a warning. Asked once per load on both paths, so this
    /// is where a mod recomputes them.
    /// Contributed files sit below the world's own data packs: a pack's file
    /// for the same path still overrides a mod's. Server-side (the single
    /// player's integrated server included). Listener applies run first. A later reload refusal
    /// preserves the old engine generation, but does not undo callback effects. Contributions
    /// share one list, so a failed file cannot be attributed to a particular mod.
    #[hook(
        side = "server",
        contract = "collect",
        out = "files",
        neoforge = "AddPackFindersEvent"
    )]
    fn data_pack_files();
}
