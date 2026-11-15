use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
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
    /// the listeners leave controls membership subject to normal eligibility.
    /// Its order controls ordinary storage targets only: crystals/dragon and
    /// players run earlier, and decorations run last in storage order. Server-side.
    pub(in crate::game) fn native_explosion_detonate(
        &mut self,
        dimension: &str,
        center: [f64; 3],
        blocks: &mut Vec<[i32; 3]>,
        entities: &mut Vec<EntityRef>,
    ) {
        self.fire_explosion_detonate(dimension, center, blocks, entities);
    }
}
