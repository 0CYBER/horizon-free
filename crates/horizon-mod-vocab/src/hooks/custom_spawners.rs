//! The custom spawner hooks: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `ModifyCustomSpawnersEvent`: the level `dimension` is about to tick its custom
    /// spawners for the first time. `spawners` is the list `ServerLevel` ticks, in order, and it
    /// starts with vanilla's (`WanderingTrader`, `Patrol`, `Cat`, `VillageSiege` and `Phantom`,
    /// in the order Horizon ticks them). Take a vanilla spawner out with `remove` and it never
    /// ticks, so its random draws are gone with it; add a spawner of the mod's own with `add` and
    /// a `SpawnerKey` the mod owns, and `custom_spawner_tick` runs for it every tick, after the
    /// ones before it in the list. A key identifies the custom spawner
    /// because an object a guest implements cannot cross to a wasm mod (Beyond NeoForge). With
    /// no listener, or one that leaves the list alone, the level ticks exactly vanilla's
    /// spawners. The phantom spawner runs after the level's mobs have ticked, as the game runs
    /// it, wherever it is in the list. It fires once per level, on the server, and listeners run on replayed ticks,
    /// so they must be pure functions of their inputs. Not cancellable.
    #[notes = r#"## Custom spawners

`modify_custom_spawners` (NeoForge's `ModifyCustomSpawnersEvent`) fires once per level, the first time the level ticks its custom spawners, with the level's `CustomSpawners` list: vanilla's five (`WanderingTrader`, `Patrol`, `Cat`, `VillageSiege`, `Phantom`) in the order Horizon ticks them. `remove` takes an entry out so it never ticks (its random draws go with it); `add` appends a mod's own spawner by a `SpawnerKey`. With no listener, or one that leaves the list alone, nothing changes: the same spawners tick in the same order.

An object a guest implements cannot cross to a wasm mod, so a mod's spawner is a key it owns and the `custom_spawner_tick(dimension, key, spawn_enemies)` hook stands for `CustomSpawner.tick`: it runs once per tick for each key in the list, in list order, while the `spawn_mobs` rule is on, and the mod that owns the key spawns through its `host`. Every mod gets the call and the others ignore it. A level with no mod spawner pays one list walk per tick and no hook call. Beyond NeoForge: a key plus a tick hook in place of an object.

Horizon ticks vanilla's spawners where it always has: the trader, patrol, cat and siege spawners in the custom spawner block (vanilla's own order differs, and moving it is a separate parity change) and the phantom spawner with the stats, after the mobs. A mod's spawners tick after them in list order, so the position of the `Phantom` entry in the list does not move it. Listeners run on replayed ticks, so they are pure functions of their inputs."#]
    #[hook(
        side = "server",
        contract = "listen",
        host = "read",
        neoforge = "ModifyCustomSpawnersEvent",
        cancellable = false
    )]
    fn modify_custom_spawners();

    /// `CustomSpawner.tick` for a spawner a mod added with `modify_custom_spawners`: the level
    /// `dimension` ticks the spawner `key`, once per tick in the list's order, with
    /// `spawn_enemies` as `ServerLevel.isSpawningMonsters` (the `spawn_monsters` rule, and not
    /// peaceful). Every mod gets the call, and the mod that owns `key` spawns through `host`
    /// (its spawns go through `finalize_spawn` and the spawn checks like any other); the rest
    /// ignore it. It is called only while the `spawn_mobs` rule is on, as vanilla's custom
    /// spawners are, and only for a key in the list, so a level with no mod spawner pays one
    /// check per tick. Beyond NeoForge: a key and this hook stand in for the `CustomSpawner`
    /// object. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "ModifyCustomSpawnersEvent"
    )]
    fn custom_spawner_tick();
}
