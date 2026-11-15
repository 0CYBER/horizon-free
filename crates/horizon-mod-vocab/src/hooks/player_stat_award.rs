//! The `player_stat_award` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0310

pub trait Hooks {
    /// A statistic is about to be awarded to a server player, before its count and scoreboard criteria
    /// change. `stat_type` and `stat` are ids such as `minecraft:mined` and `minecraft:stone`, and
    /// `value` the amount. All three may be rewritten; ids must use their canonical namespaced
    /// spelling. Unknown types or entries, noncanonical ids and negative amounts are refused
    /// together, warning once and keeping the entire original award.
    /// Set `cancel` to award nothing. Server-side. Ordinary interactions
    /// fire at vanilla's award point; inside a transaction, the award and this hook wait until
    /// the outermost successful commit, after tentative world and inventory writes.
    #[hook(
        version = 2,
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "StatAwardEvent",
        cancellable = true
    )]
    fn player_stat_award();
}
