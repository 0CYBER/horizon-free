//! A bed's head half and the world border.
use super::*;
use horizon_core::geometry::border::BorderGeometry;

/// `AbstractBedBlock.getStateForPlacement` refuses a bed whose head half
/// (`relative`, one block the way the player faces) is outside the border
/// (`WorldBorder.isWithinBounds(BlockPos)`, the half-open x/z box). A size 2
/// border centred on the origin holds x and z in [-1, 1): looking south from
/// the foot at (0, 1, 0), the head would land on z = 1, so nothing places.
/// A size 4 border holds z = 1 and the same click places both halves.
#[test]
fn a_bed_whose_head_is_outside_the_world_border_is_refused() {
    let mut w = world!();
    w.floor(0, 4, "stone");
    let ground = BlockPos::new(0, 0, 0);
    let foot = BlockPos::new(0, 1, 0);
    let head = BlockPos::new(0, 1, 1);
    let air = w.level.air;

    w.level.border = BorderGeometry::new([0.0, 0.0], 2.0, 29_999_984).expect("a two block border");
    let outcome = w.place("white_bed", ground, Direction::Up, 0.0, 30.0, 0.0);
    assert!(matches!(outcome, PlaceOutcome::Failed), "{outcome:?}");
    assert_eq!(w.get(foot), air, "the foot is refused");
    assert_eq!(w.get(head), air, "the head is refused");

    w.level.border = BorderGeometry::new([0.0, 0.0], 4.0, 29_999_984).expect("a four block border");
    let outcome = w.place("white_bed", ground, Direction::Up, 0.0, 30.0, 0.0);
    assert!(
        matches!(outcome, PlaceOutcome::Placed { pos, .. } if pos == foot),
        "{outcome:?}"
    );
    assert_eq!(w.prop(foot, "part"), "foot");
    assert_eq!(w.prop(foot, "facing"), "south");
    assert_eq!(w.prop(head, "part"), "head");
}
