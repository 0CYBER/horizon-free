//! `BlockBehaviour.useWithoutItem`/`attack` beyond what the interaction tests
//! in `tests.rs` cover, and the placement a used block item asks for.
use super::*;
use horizon_core::geometry::border::BorderGeometry;

/// `DragonEggBlock.teleport` only picks a position inside the border
/// (`WorldBorder.isWithinBounds`): while the border holds none of the cells
/// the egg may pick, it stays where it is, with no level event and no block
/// change; around the egg it moves as before.
#[test]
fn a_dragon_egg_teleports_only_inside_the_world_border() {
    let mut w = world!();
    // A floor wide enough that the air and ground checks accept most picks,
    // so only the border can keep the egg in place below.
    w.floor(5, 16, "stone");
    let pos = BlockPos::new(0, 6, 0);
    let egg = w.id("dragon_egg", &[]);
    w.put(pos, egg);
    // One block wide, far from the egg: every cell it can pick (within 16 of
    // it in x and z) is outside, though air, ground and build height accept it.
    w.level.border =
        BorderGeometry::new([1000.0, 1000.0], 1.0, 29_999_984).expect("a one block border");
    w.b().attack_block(&mut w.level, &mut w.ticks, pos);
    assert_eq!(w.get(pos), egg);
    assert!(
        !w.level
            .events
            .iter()
            .any(|e| matches!(e, LevelEvent::LevelEvent { id: 2015, .. }))
    );

    // Around the egg every pick is inside, so it teleports as before.
    w.level.border = BorderGeometry::new([0.0, 0.0], 64.0, 29_999_984).expect("a 64 block border");
    w.b().attack_block(&mut w.level, &mut w.ticks, pos);
    assert_ne!(w.get(pos), egg);
    assert!(
        w.level
            .events
            .iter()
            .any(|e| matches!(e, LevelEvent::LevelEvent { id: 2015, .. }))
    );
}

/// `BlockState.canSurvive` for a cactus flower standing at `pos`.
fn survives(w: &mut World<'_>, pos: BlockPos) -> bool {
    let flower = w.id("cactus_flower", &[]);
    w.b()
        .sim(&mut w.level, &mut w.ticks)
        .can_survive(flower, pos)
}

/// `CactusFlowerBlock.mayPlaceOn` asks `isFaceSturdy(level, pos, UP, CENTER)`
/// of the live block, so a shulker box answers with
/// `ShulkerBoxBlock.getBlockSupportShape`: an open box is the one-pixel plate
/// opposite `facing`, not the closed full cube a shape cache would give.
#[test]
fn a_cactus_flower_follows_a_shulker_boxs_live_support_shape() {
    let mut w = world!();
    let box_pos = BlockPos::new(0, 1, 0);
    let flower_pos = BlockPos::new(0, 2, 0);
    let flower = w.id("cactus_flower", &[]);
    let place = |w: &mut World<'_>, face: Direction| {
        let ctx = PlaceContext {
            clicked_pos: box_pos,
            face,
            hit: horizon_core::geometry::Vec3::new(0.5, 2.0, 0.5),
            inside: false,
            yaw: 0.0,
            pitch: 0.0,
            sneaking: false,
            item: "minecraft:cactus_flower".into(),
            can_use_game_master_blocks: false,
        };
        let outcome = w.b().place_block(&mut w.level, &mut w.ticks, &ctx);
        (outcome, w.get(flower_pos))
    };

    // A shut box is a full cube.
    w.put(box_pos, w.id("shulker_box", &[("facing", "up")]));
    assert!(
        survives(&mut w, flower_pos),
        "a closed box holds the flower"
    );
    assert_eq!(
        place(&mut w, Direction::Up).0,
        PlaceOutcome::Placed {
            pos: flower_pos,
            state: flower,
        }
    );

    // Opened facing up, the plate is the bottom one: the face above it is
    // empty, so the flower is refused and nothing is written.
    w.put(flower_pos, w.b().air());
    w.level.shulker_closed.insert(box_pos, false);
    assert!(
        !survives(&mut w, flower_pos),
        "an open box's empty lid holds no cactus flower"
    );
    let (outcome, at) = place(&mut w, Direction::Up);
    assert_eq!(outcome, PlaceOutcome::Failed);
    assert_eq!(at, w.b().air(), "nothing was placed");

    // A box opening down keeps its plate on top, and that face reads as a
    // full square, so the flower stands.
    w.put(box_pos, w.id("shulker_box", &[("facing", "down")]));
    w.level.shulker_closed.insert(box_pos, false);
    assert!(
        survives(&mut w, flower_pos),
        "the base plate of a box opening down holds the flower"
    );

    // A flower placed on the shut box pops when the lid opens.
    w.put(box_pos, w.id("shulker_box", &[("facing", "up")]));
    w.level.shulker_closed.remove(&box_pos);
    assert!(w.set(flower_pos, flower), "the flower is placed on the box");
    w.level.events.clear();
    w.level.shulker_closed.insert(box_pos, false);
    let lid = w.get(box_pos);
    w.b()
        .shulker_neighbor_updates(&mut w.level, &mut w.ticks, box_pos, lid);
    assert_eq!(
        w.get(flower_pos),
        w.b().air(),
        "the flower pops off the opened lid"
    );
    assert!(
        w.level.events.iter().any(
            |e| matches!(e, LevelEvent::DropResources { pos, state } if *pos == flower_pos && *state == flower)
        ),
        "the popped flower drops itself: {:?}",
        w.level.events
    );
}
