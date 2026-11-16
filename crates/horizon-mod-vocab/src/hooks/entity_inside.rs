//! The `entity_inside` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// An entity's block sweep crossed a block state this part is attached to (`Block.entityInside`), and the part is told mid-sweep, where vanilla runs the block's own `entityInside`. `event.precise` is `Entity.checkInsideBlocks`' flag: the entity moved nearly a block, or its box at the target touches the block. It runs inside the entity's move, on the level random host: it reads blocks and draws the level's random in vanilla's order, interleaved with the vanilla blocks the same walk visits; only writes are restricted to the event. `event.apply` queues one of the five `InsideBlockEffectType`s, `event.run_before`/`event.run_after` queue a typed entity write around one, and `event.request` asks for a level write that lands at the touched block once the entity's batch has ticked, if the block is still the state it touched — the same contract as a landing write, whose complete batch is checked before queuing and an invalid state refuses all writes and sibling outputs. The queued entity writes apply to the swept entity inside its own tick, in `StepBasedCollector` order, and stop when it dies; a `Hurt` whose source the entity table answers `Taken` for (its list is lifted that moment, ) is refused with a typed error, not a retry and not a panic. Fires for players, mobs, items and projectiles alike, each on the shared walk inside its own tick or batch.
    #[hook(
        side = "server",
        contract = "collect",
        out = "effects",
        scope = "block",
        key = "state",
        host = "level",
        target = "horizon_client::game::native_hooks::entity_inside::HitLevel<'_>::native_entity_inside",
        neoforge = "none (vanilla Block.entityInside; NeoForge adds no event or extension for it)"
    )]
    fn entity_inside(
        dimension: &str,
        entity: i32,
        kind: &str,
        state: BlockState,
        pos: BlockPos,
        precise: bool,
        effects: &mut InsideEffects,
        writes: &mut Vec<InsideWrite>,
    );
}
