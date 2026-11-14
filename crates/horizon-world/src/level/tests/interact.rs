//! `BlockBehaviour.useWithoutItem`/`attack` beyond what the interaction tests
//! in `tests.rs` cover.
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
