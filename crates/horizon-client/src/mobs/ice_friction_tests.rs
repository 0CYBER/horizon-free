//! `LivingEntity.travelInAir` reads the friction of the block at
//! `Entity.getBlockPosBelowThatAffectsMyMovement` (`Entity.getOnPos(0.500001F)`),
//! which is the *supporting* block's column (`mainSupportingBlockPos`), not the
//! column the mob's centre happens to be over. A cow whose centre is over
//! farmland (15/16 tall, so its shape does not reach the feet) while its box
//! rests on the blue ice beside it slides on the ice.
use super::*;
use std::sync::Arc;

/// Farmland under the cow's centre, blue ice under the rest of its box.
fn edge_of_blue_ice(
    world: &mut crate::world::World,
    registry: &horizon_world::registry::BlockRegistry,
) {
    world
        .chunks
        .insert((0, 0), Arc::new(crate::chunk::Chunk::empty(0, 0)));
    let stone = registry.default_state("minecraft:stone").unwrap();
    for x in 0..4 {
        for z in 0..4 {
            world.set(x, 0, z, stone).unwrap();
        }
    }
    world
        .set(
            0,
            0,
            0,
            registry.default_state("minecraft:farmland").unwrap(),
        )
        .unwrap();
    world
        .set(
            1,
            0,
            0,
            registry.default_state("minecraft:blue_ice").unwrap(),
        )
        .unwrap();
}

#[test]
fn a_cow_slides_on_the_supporting_blocks_friction() {
    let ran = crate::mobs::world::with_built_test_world(&[], edge_of_blue_ice, |w, types| {
        let kind = Arc::new(types.types["minecraft:cow"].clone());
        let mut cow = Mob::new(1, kind, Vec3::new(0.9, 1.0, 0.5), 0.0);
        cow.on_ground = true;
        // `Entity.checkSupportingBlock`: the ice's full shape reaches the feet,
        // the farmland's 15/16 does not.
        cow.supporting = cow.supporting_block(w, Vec3::ZERO);
        assert_eq!(cow.supporting, Some([1, 0, 0]), "the ice supports it");
        assert_eq!(cow.block_friction(w), 0.989, "the ice's friction");
        // One tick's slide with no input: 0.5 times the ice's 0.989 and the
        // 0.91 air drag.
        cow.velocity = Vec3::new(0.5, 0.0, 0.0);
        cow.travel_in_air(w, Vec3::ZERO, &mut Vec::new());
        assert!(
            (cow.velocity.x - 0.449995).abs() < 1.0e-6,
            "{}",
            cow.velocity.x
        );
    });
    let _ = ran;
}
