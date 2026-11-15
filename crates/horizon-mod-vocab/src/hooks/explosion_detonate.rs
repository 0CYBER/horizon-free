//! The `explosion_detonate` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0042

pub trait Hooks {
    /// An explosion at `center` in `dimension` (`minecraft:overworld`) is
    /// about to break `blocks` and reach `entities`: `ServerExplosion.explode`
    /// with its `toBlow`, before any block or entity is hit. Take out the
    /// positions to spare; they keep their block and get no fire. `entities`
    /// is `getEntities(source, box)`: every entity in the box the centre
    /// ± (2 × radius + 1), in walk order, without the exploding source, those
    /// that will ignore the blast included — the entity-list walk, then the
    /// item frames, paintings, cushions and end crystals it misses. Take one
    /// out and it is neither hurt nor pushed, and `explosion_knockback` is
    /// not asked for it; one a mod adds is walked as any other, and the list
    /// the listeners leave controls membership, subject to the usual distance,
    /// exposure and per-kind immunity checks. Order controls the ordinary-storage
    /// walk only, not global effect order: crystals/dragon and players run in
    /// separate earlier passes, and decorations run last in storage order.
    /// Block positions are shuffled before destruction, so their list order is
    /// not destruction order either. Server-side.
    #[hook(
        side = "server",
        contract = "collect",
        out = "blocks",
        version = 2,
        neoforge = "ExplosionEvent.Detonate"
    )]
    fn explosion_detonate();
}
