//! The `mob_spawned` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0048

pub trait Hooks {
    /// A new mob of `kind` joined `dimension` at `pos`: `natural` when the
    /// world's spawning made it, false when something else did (a breed, a
    /// raid, a spawner, a conversion, `/summon`, and the second mob of a
    /// jockey the world spawned). One call a mob: a `/summon` with passengers
    /// tells each. Not told of a mob loaded with its chunk, leaving a hive or
    /// changing level. It only hears; the mob is already among the game's
    /// mobs. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "EntityJoinLevelEvent, FinalizeSpawnEvent"
    )]
    fn mob_spawned();
}
