//! The `mob_spawn_checks` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `MobSpawnEvent.SpawnPlacementCheck`: may the entity type `kind` spawn at `pos` in `level` for `reason`? It is asked after vanilla's own spawn predicate (`SpawnPlacements.checkSpawnRules`: light, the ground it needs, the biome, slime chunks, the peaceful rule) has run: `default_result` is what it answered, and `result` starts as `Default`, which keeps that answer. Set `Succeed` or `Fail` to force the check. It runs wherever the game calls `checkSpawnRules` with the game's level: natural spawning, a village siege's zombies, a patrol, a zombie's reinforcements, a spawner and a trial spawner. `level` is the level the mob would spawn in (`dimension` names it too), so a listener can read the blocks around `pos` through the host. `spawner` is the block that asked: `Spawner::Block` for a spawner block, `Spawner::Trial` for a trial spawner, and `None` for natural spawning, sieges, patrols, reinforcements and a spawner minecart. A spawner whose spawn data carries custom spawn rules never reaches it. The predicate always runs first, as `default_result` is its answer, so its random draws are vanilla's whatever a mod answers; a listener should be a pure function of its inputs, because the checks run again when a tick is replayed. The host is the read-only one: a listener reads the world and writes nothing. Not told of `/summon` or a spawn egg, which do not check placements. Server-side. Beyond NeoForge: one `SpawnCheck` answers this event and `mob_spawn_position_check`, which NeoForge gives two nested enums with the same three constants.
    #[notes = r#"## Spawn checks

- **`mob_spawn_placement_check(dimension, kind, reason, pos, default_result, level, spawner, result)`** (NeoForge's `MobSpawnEvent.SpawnPlacementCheck`) and **`mob_spawn_position_check(dimension, kind, reason, pos, level, spawner, mob, result)`** (`MobSpawnEvent.PositionCheck`): set `result` to `SpawnCheck::Succeed` or `Fail` to force a spawn check; `Default` keeps vanilla's. The placement check runs after the type's spawn predicate (`default_result` is its answer), so its random draws are vanilla's whatever a mod answers. The position check stands in for `Mob.checkSpawnRules` and `checkSpawnObstruction`: with no listener or `Default` they run and draw as vanilla, while `Succeed` and `Fail` skip them and their draws. Fired wherever the game checks a spawn: natural spawning, chunk generation, spawners, trial spawners (obstruction only), patrols, sieges and reinforcements for the placement check. Both get `level` (`LevelRef`, the level the mob would join) and `spawner` (`Option<Spawner>`, `#[non_exhaustive]`: `Block { pos }` for a spawner block, `Trial { pos }` for a trial spawner, `None` where no block asked: natural spawning, chunk generation, patrols, sieges, reinforcements and a spawner minecart). The position check also gets `mob` (`UnspawnedMobRef`), the mob itself: vanilla creates the mob before this check, and Horizon, which otherwise creates it after, creates it before the listeners only when the hook is listened, the way the site creates it, and hands that same mob on to the spawn, so the mob a listener read is the one that spawns; a refused check drops it, never added to the level, and gives its provisional id back. Creating it draws nothing from the level's random, so a listener that answers `Default` leaves every draw as an unlistened spawn's. Its reads are `finalize_spawn`'s (`kind`, `position`, `equipment`, `attribute_base`, `attribute_value`, `tags`, `custom_name`); there is no editor, as the hook only reads. Where the site does not create the mob fresh, the handle answers `kind` and `position` and `Unsupported` for the rest: chunk generation's packs (the generator creates and finalizes them with its own random, after the check) and a spawner or trial spawner whose spawn data carries entity data (the game loads it from that data after the tick). Both get the read-only host and run on replayed ticks, so a listener is a pure function of its inputs. Tick-time effect: one `listened` branch with no listeners, before any `SpawnReason` is built; with a position-check listener, the mob is created one step early, not twice. One `SpawnCheck` serves both (NeoForge's two enums have the same constants)."#]
    #[hook(
        side = "server",
        contract = "listen",
        host = "read",
        neoforge = "MobSpawnEvent.SpawnPlacementCheck",
        cancellable = false
    )]
    fn mob_spawn_placement_check(
        dimension: &str,
        kind: &str,
        reason: SpawnReason,
        pos: BlockPos,
        default_result: bool,
        level: LevelRef,
        spawner: Option<Spawner>,
        result: &mut SpawnCheck,
    );
    /// NeoForge's `MobSpawnEvent.PositionCheck`: may the freshly created mob of type `kind` stand at `pos` (its position, a mob's centre) in `level` for `reason`? Vanilla's two checks are `Mob.checkSpawnRules` (the type's own rules) and `Mob.checkSpawnObstruction` (inside blocks or fluids, an ocelot's grass and sea level). `result` starts as `Default`, which runs them; `Succeed` or `Fail` answers instead and they are skipped. It runs on natural spawning, chunk generation's creature packs and spawner blocks (where a spawner with custom spawn rules skips `Mob.checkSpawnRules` on `Default`, as vanilla does).  A trial spawner runs only `checkSpawnObstruction`, so only that is skipped. `spawner` is the block that asked (`Spawner::Block` or `Spawner::Trial`), `None` for natural spawning, chunk generation and a spawner minecart. `mob` is the mob itself, in no level yet: read it through the host (`mob.kind(host)`, `mob.position(host)`, `mob.equipment(host, slot)`); after the hook returns the handle answers `Expired`. It is the mob that spawns if the check passes, handed on rather than created again, and dropped without trace if it fails. Where the game does not create the mob fresh (chunk generation's packs, a spawner whose spawn data carries entity data) the handle answers its kind and position and `Unsupported` for the rest. Server-side. With no listener or `Default` the checks and their random draws are vanilla's (creating the mob for the listener draws nothing from the level's random); `Succeed` and `Fail` skip the checks and so their draws, and the level random's order changes only then; a listener should be a pure function of its inputs, because ticks are replayed.
    #[hook(
        side = "server",
        contract = "listen",
        host = "read",
        neoforge = "MobSpawnEvent.PositionCheck",
        cancellable = false
    )]
    fn mob_spawn_position_check(
        dimension: &str,
        kind: &str,
        reason: SpawnReason,
        pos: [f64; 3],
        level: LevelRef,
        spawner: Option<Spawner>,
        mob: UnspawnedMobRef,
        result: &mut SpawnCheck,
    );
}
