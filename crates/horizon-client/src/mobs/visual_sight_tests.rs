//! `LivingEntity.isLookingAtMe(.., seeThroughTransparentBlocks = true)` clips
//! with `ClipContext.Block.VISUAL` (`LivingEntity.java:1771`, `Creaking.java:471`),
//! i.e. `BlockBehaviour.getVisualShape` (`BlockBehaviour.java:334`), not the
//! collision shape: `PowderSnowBlock.getVisualShape` is empty,
//! `FenceBlock.getVisualShape` is its 16-pixel `getShape` and not its 24-pixel
//! collision shape, and `SnowLayerBlock.getVisualShape` is one layer taller
//! than its collision shape.
use super::*;
use std::sync::Arc;

/// `visual_clear_between` and `clear_between` (the VISUAL and the COLLIDER
/// clip) for a segment through the block at (0, 0, 0): (seen through, walked
/// through).
fn ray(block: &str, properties: &[(&str, &str)], from: Vec3, to: Vec3) -> Option<(bool, bool)> {
    let mut out = None;
    let ran = with_built_test_world(
        &[],
        |world, registry| {
            let mut state = registry
                .state(registry.default_state(block).unwrap())
                .unwrap()
                .clone();
            for (key, value) in properties {
                state.properties.insert((*key).into(), (*value).into());
            }
            let state = registry.id(&state).unwrap();
            world
                .chunks
                .insert((0, 0), Arc::new(crate::chunk::Chunk::empty(0, 0)));
            world.set(0, 0, 0, state).unwrap();
        },
        |w, _| {
            out = Some((
                w.visual_clear_between(from, to),
                w.clear_between(from, to, false),
            ));
        },
    );
    let _ = ran;
    out
}

/// The same for a ray along +z through the block at (0, 0, 0) at `height`.
fn flat(block: &str, properties: &[(&str, &str)], height: f64) -> Option<(bool, bool)> {
    ray(
        block,
        properties,
        Vec3::new(0.5, height, -0.5),
        Vec3::new(0.5, height, 0.5),
    )
}

/// A player behind powder snow is seen through it at every height.
#[test]
fn powder_snow_is_seen_through() {
    for height in [0.1, 0.5, 1.2, 1.4] {
        let Some((seen, _)) = flat("minecraft:powder_snow", &[], height) else {
            return;
        };
        assert!(seen, "seen through powder snow at y = {height}");
    }
}

/// A four-layer snow block's outline reaches 0.5, its collision shape only
/// 0.375: the visual ray stops in between where the collider does not.
#[test]
fn snow_hides_what_its_collision_shape_does_not() {
    let Some((seen, walked)) = flat("minecraft:snow", &[("layers", "4")], 0.45) else {
        return;
    };
    assert!(!seen, "the outline reaches 0.5");
    assert!(walked, "the collision shape only reaches 0.375");
    let Some((seen, walked)) = flat("minecraft:snow", &[("layers", "4")], 0.6) else {
        return;
    };
    assert!(seen && walked, "both shapes are below 0.6");
}

/// A fence's 24-pixel collision shape is taller than its 16-pixel outline.
/// The extra height lies in the cell above the fence, which
/// `BlockGetter.traverseBlocks` only queries for a segment that leaves that
/// cell: a flat ray over the post misses both shapes, a descending one is
/// stopped by the collision shape where vanilla's visual ray passes.
#[test]
fn a_fence_is_seen_through_above_its_post() {
    for height in [1.2, 1.45] {
        let Some((seen, walked)) = flat("minecraft:oak_fence", &[], height) else {
            return;
        };
        assert!(seen && walked, "no ray queries the fence at y = {height}");
    }
    let Some((seen, walked)) = flat("minecraft:oak_fence", &[], 0.5) else {
        return;
    };
    assert!(!seen && !walked, "the post blocks both rays");
    let from = Vec3::new(0.5, 1.2, 0.3);
    let to = Vec3::new(0.5, 0.95, 0.9);
    let Some((seen, walked)) = ray("minecraft:oak_fence", &[], from, to) else {
        return;
    };
    assert!(seen, "the outline stops at y = 1.0");
    assert!(!walked, "the collision shape reaches y = 1.5");
}

/// A full block is the default: seen through by neither ray.
#[test]
fn a_full_block_blocks_sight() {
    for height in [0.5, 0.9] {
        let Some((seen, walked)) = flat("minecraft:stone", &[], height) else {
            return;
        };
        assert!(!seen && !walked, "stone blocks sight at y = {height}");
    }
}
