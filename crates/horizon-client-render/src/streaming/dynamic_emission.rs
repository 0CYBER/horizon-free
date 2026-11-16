//! Whether a light job's 3x3 is all there, and asked: a chunk that came in holding a
//! `dynamic_light` state waits for the game to ask its cells (`Chunk::dynamic_pending`).
use super::Proto;
use crate::{
    fast_hash::{FxHashMap, FxHashSet},
    world::World,
};

/// Whether the 3x3 a light job for (x, z) reads is all there: every
/// neighbour a finished column whose `dynamic_light` cells are asked, or a
/// decorated proto chunk past the area (the last FULL ring lights against
/// neighbours at FEATURES, as `Streaming::light_region` reads them).
pub(super) fn light_region_ready(
    world: &World,
    in_range: impl Fn(i32, i32) -> bool,
    decorated: &FxHashSet<(i32, i32)>,
    protos: &FxHashMap<(i32, i32), Option<Proto>>,
    x: i32,
    z: i32,
) -> bool {
    for dz in 0..3 {
        for dx in 0..3 {
            let n = (x + dx - 1, z + dz - 1);
            if world.chunks.get(&n).is_some_and(|c| !c.dynamic_pending)
                || (!in_range(n.0, n.1)
                    && decorated.contains(&n)
                    && matches!(protos.get(&n), Some(Some(_))))
            {
                continue;
            }
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::Chunk;
    use std::sync::Arc;

    /// A neighbour whose `dynamic_light` cells are not asked yet holds every light job that
    /// reads it, and lets them through once the game has asked.
    #[test]
    fn a_neighbour_waiting_to_be_asked_holds_the_light_job() {
        let mut world = World::new(horizon_world::registry::BlockStateId(0), 0);
        for dz in -1..=1 {
            for dx in -1..=1 {
                world
                    .chunks
                    .insert((dx, dz), Arc::new(Chunk::empty(dx, dz)));
            }
        }
        let none = FxHashSet::default();
        let protos = FxHashMap::default();
        let ready = |world: &World| light_region_ready(world, |_, _| true, &none, &protos, 0, 0);
        assert!(ready(&world));
        let corner = world.chunks.get_mut(&(1, 1)).unwrap();
        Arc::make_mut(corner).dynamic_pending = true;
        assert!(!ready(&world));
        Arc::make_mut(world.chunks.get_mut(&(1, 1)).unwrap()).dynamic_pending = false;
        assert!(ready(&world));
    }
}
