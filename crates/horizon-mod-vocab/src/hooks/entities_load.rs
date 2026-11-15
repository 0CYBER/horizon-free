//! The `entities_load` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0196

pub trait Hooks {
    /// A server player's or mob's ender pearl is about to teleport its `owner`
    /// (`EntityTeleportEvent.EnderPearl`, `ThrownEnderpearl.onHit`). `owner` is
    /// the live entity handle, `pearl` its projectile id, `from` the owner's feet
    /// and `target` the pearl's previous position. A mod may move the target;
    /// non-finite changes keep the original spot. Canceling leaves the owner
    /// in place with no teleport sound, damage or endermite roll; the pearl is
    /// used up. Player owners keep their acting-player context and mutable fall
    /// `damage` (5; nonpositive causes no health loss, but zero remains a hit
    /// attempt that asks the invulnerability predicate). Mob owners take no fall damage and
    /// make no endermite draw; their `damage` starts at zero and is player-only.
    /// An owner moved, removed, killed or put to sleep by a listener is not moved
    /// again using the old decision. The impact hook still hears the hit.
    /// What the pearl hit (`getHitResult`), in `projectile_impact`'s shape:
    /// the entity `hit_entity` (a dragon part is the dragon), or, with it
    /// None, the block `hit_block` on its `hit_face` (the face's normal); `hit_at`
    /// is the hit point. For an entity, `hit_block` is the cell of `hit_at`
    /// and `hit_face` is `[0, 0, 0]`.
    #[hook(
        side = "server",
        contract = "cancel",
        version = 3,
        out = "cancel",
        neoforge = "EntityTeleportEvent.EnderPearl",
        cancellable = true
    )]
    fn ender_pearl_teleport(
        owner: EntityRef,
        pearl: u32,
        from: [f64; 3],
        target: &mut [f64; 3],
        damage: &mut f32,
        cancel: &mut bool,
        hit_entity: Option<u32>,
        hit_block: [i32; 3],
        hit_face: [i32; 3],
        hit_at: [f64; 3],
    );
    /// Entities from `chunk` finished loading into the server world. `count`
    /// is the number of saved entity records read.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn entities_load();
}
