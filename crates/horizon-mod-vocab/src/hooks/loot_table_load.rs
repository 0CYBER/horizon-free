//! The `loot_table_load` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0087

pub trait Hooks {
    /// A native mod's hook point (docs/modding/src/how-to/native-mods.md), declared in `hooks/*.rs` with the game's:
    /// the loot table `id` (`minecraft:chests/simple_dungeon`, `minecraft:blocks/stone`,
    /// `minecraft:entities/zombie`) as it loads, its JSON after the data packs', before anything
    /// rolls it (NeoForge's `LootTableLoadEvent`, Fabric's `LootTableEvents.MODIFY`). Add pools to
    /// `table["pools"]` or entries to a pool's `entries`, change or replace it (no pools: it drops
    /// nothing; keep its `random_sequence` when replacing it, as a mob's table without one drops
    /// nothing). Called once per table in id order when the server loads or reloads its data, with the
    /// `DataLoadContext` of that owning load (`origin()` is always `ServerDataLoad`: the client never loads
    /// loot tables, so the hook never fires there).
    #[hook(
        side = "server",
        contract = "collect",
        out = "table",
        context = "data_load",
        version = 2,
        target = "horizon_world::loot::native_loot_table_load",
        neoforge = "LootTableLoadEvent"
    )]
    fn loot_table_load();
}
