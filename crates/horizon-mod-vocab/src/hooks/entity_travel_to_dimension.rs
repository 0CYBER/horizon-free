//! The `entity_travel_to_dimension` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `entity` is about to travel to the level `dimension` (NeoForge's
    /// `EntityTravelToDimensionEvent`), before anything of it moves: the
    /// entity is still where it is, in its own level. `cause` is why:
    /// `nether_portal`, `end_portal` or `end_gateway` for a portal, or
    /// `teleport` for a teleport that changes level (`/tp` and `/teleport`,
    /// a spectator's teleport to a player in another level). Set `cancel` to
    /// keep it where it is: a portal's wait is forgotten, a teleport is
    /// dropped and `/tp` still reports success. An End gateway leads to
    /// the level the entity is already in, so its `dimension` is the entity's
    /// own.
    ///
    /// Asked for server players, and for the other entities that cross a
    /// portal or a command's teleport: items, mobs, boats, minecarts, arrows
    /// and other projectiles, experience orbs, primed TNT, tridents,
    /// fireworks and falling blocks. A vehicle and its passengers are asked
    /// recursively. A refused passenger stays in the source level, dismounted,
    /// with its own passengers; it does not veto the vehicle's travel. An entity
    /// discarded during a callback never travels. A falling block is held out of the level while it
    /// ticks, so its handle reads nothing yet. A respawn and a world's join
    /// are not travel; the return from the End credits is `end_portal`.
    /// Server-side. Diverges
    /// from upstream: replaces `player_portal`, which covered only server
    /// players through portals.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "EntityTravelToDimensionEvent",
        cancellable = true
    )]
    fn entity_travel_to_dimension();
}
