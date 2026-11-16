// Test module extracted without changing its logical path.
use super::*;
use crate::pose::SizeAnswer;

/// `EntityEvent.Size`: an `entity_size` answer replaces the mob's dimensions while its
/// pose and vanilla size hold; a change to either goes back to vanilla's.
#[test]
fn a_size_answer_holds_until_the_pose_or_vanilla_size_changes() {
    let mut cow = Mob::new(1, crate::mobs::sensor_scan_tests::kind(), Vec3::ZERO, 0.0);
    let vanilla = cow.vanilla_dims();
    assert_eq!(dims_array(cow.dims()), vanilla);
    cow.size_answer = Some(SizeAnswer {
        pose: cow.pose(),
        vanilla,
        size: Some([2.0, 3.0, 2.5]),
    });
    assert_eq!(cow.dims(), (2.0, 3.0, 2.5));
    assert_eq!(
        (cow.width(), cow.height(), cow.eye_height()),
        (2.0, 3.0, 2.5)
    );
    assert_eq!(cow.vanilla_dims(), vanilla);
    // A baby's vanilla size is another one: the answer no longer holds.
    crate::mobs::finalize::set_baby(&mut cow, true);
    assert_ne!(cow.vanilla_dims(), vanilla);
    assert_eq!(dims_array(cow.dims()), cow.vanilla_dims());
    // An answer that left vanilla's size changes nothing.
    cow.size_answer = Some(SizeAnswer {
        pose: cow.pose(),
        vanilla: cow.vanilla_dims(),
        size: None,
    });
    assert_eq!(dims_array(cow.dims()), cow.vanilla_dims());
}
