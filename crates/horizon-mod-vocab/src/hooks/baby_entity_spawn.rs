//! The `baby_entity_spawn` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Two animals bred and a baby is about to be made (NeoForge's
    /// `BabyEntitySpawnEvent`, vanilla's `Animal.spawnChildFromBreeding`):
    /// `parent` and `partner` are entity ids, `player` and `player_entity_id`
    /// the UUID and entity id of the player who fed them into love
    /// (`getCausedByPlayer`, None when none), `kind` the baby's type (a horse
    /// and a donkey have a mule). NeoForge's `setChild` (another baby)
    /// needs a handle to a mob not yet in the world, which this hook does not expose. Set `cancel` for no baby: the
    /// parents still rest and lose their love (their ages were already set
    /// to 6000), but there is no baby, no experience, no `animals_bred`
    /// statistic, no advancement and no heart particles. Not fired for
    /// turtles, frogs and sniffers (an egg, no baby) or armadillos' own paths.
    /// Villagers do not post this animal-only event, matching NeoForge's
    /// documented `Animal.spawnChildFromBreeding` posting site; a rejected
    /// villager join instead leaves the child's bed free. On the server.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "BabyEntitySpawnEvent",
        cancellable = true
    )]
    fn baby_entity_spawn();
}
