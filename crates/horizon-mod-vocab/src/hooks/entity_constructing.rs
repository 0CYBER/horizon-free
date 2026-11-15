//! The `entity_constructing` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A new entity is being made (NeoForge's `EntityEvent.EntityConstructing`, vanilla's
    /// `Entity.<init>`): `kind` is its type (`minecraft:zombie`, `minecraft:player` for a
    /// player) and `holder` holds its attachments. Set attachments here and the entity starts
    /// with them: this runs before any mod reads them, and an entity read from disk or from a
    /// summon's NBT reads its saved attachments over what you set. The entity may not be in
    /// the level yet, so read nothing else of it. Nothing can be cancelled.
    ///
    /// Told once per entity on each side that makes it, as vanilla's server and client each
    /// construct their own: on the server (single player's included) for a spawned, summoned,
    /// bred or loaded entity and for a guest's player when it joins and when it respawns; on
    /// a client for each entity and player its server adds to its view, and for its own
    /// player at login and at each respawn, a dimension change's included (vanilla makes a
    /// new `LocalPlayer` there). A guest's file, or what its player keeps through a respawn,
    /// is read over what you set. An entity whose chunk unloads and loads again is a new
    /// entity, told again, as in vanilla. An entity changing dimension keeps its attachments
    /// and is not told again, where vanilla constructs the copy it moves
    /// (`Entity.teleportCrossDimension`, then `restoreFrom`).
    #[hook(
        side = "both",
        contract = "listen",
        neoforge = "EntityEvent.EntityConstructing"
    )]
    fn entity_constructing();
}
