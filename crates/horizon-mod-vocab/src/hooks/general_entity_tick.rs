//! The `general_entity_tick` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The authoritative tick of one general entity row of type `kind`
    /// (`mymod:robot`), with ID `entity`, in the server level's entity phase:
    /// `ServerLevel.tickNonPassenger`'s `entity.tick()` for a row the type
    /// registry owns (arismoko/horizon#792 stage 4). The row's common bookkeeping has run
    /// (its previous position is its current one); a rider is ticked after
    /// its vehicle inside the vehicle's own tick, depth-first as
    /// `tickPassenger` recurses, and is never ticked as a root. Rows that
    /// joined this tick tick from the next, parked levels and rows outside
    /// entity-ticking chunks do not tick, and a `entity_tick_pre` that
    /// cancels the row cancels its whole rider subtree. Read and change the
    /// row's data through its attachments (`event.entity` is their holder);
    /// there is no parallel per-entity state. Told only for general entity
    /// types: vanilla families tick their own paths (#6509 owns bringing
    /// them to one join-order walk), so a part attached to `minecraft:zombie`
    /// hears no tick from this hook. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        scope = "entity",
        key = "kind",
        neoforge = "none (vanilla Entity.tick has no part-scoped counterpart; NeoForge's EntityTickEvent is the global pre/post pair)"
    )]
    fn general_entity_tick();
}
