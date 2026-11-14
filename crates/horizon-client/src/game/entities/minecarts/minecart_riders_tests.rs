//! `AbstractMinecart`'s rider seat and dismount side: a villager or wandering
//! trader sits at the cart's own y, and the old behavior sets a rider down
//! beside its motion direction, not beside its facing.
use super::*;

/// `AbstractMinecart.getPassengerAttachmentPoint` lowers a villager's and a
/// wandering trader's seat to the cart's own y; any other rider keeps the
/// cart's 0.1875 attachment.
#[test]
fn a_villager_sits_on_the_cart_itself() {
    let at = Vec3::new(1.0, 10.0, 2.0);
    for species in [Species::Villager, Species::WanderingTrader] {
        assert_eq!(cart_attachment(species), 0.0, "{species:?}");
        // Neither has a `ridingOffset` of its own.
        let seat = cart_seat(at, cart_seat_offset(species, 0.0));
        assert!(
            (seat.y - 10.0).abs() < 1e-12,
            "{species:?} sits at {seat:?}"
        );
    }
    // A cow keeps the cart's 0.1875, less a `ridingOffset` if it had one.
    let cow = cart_seat(at, cart_seat_offset(Species::Cow, 0.0));
    assert!((cow.y - 10.1875).abs() < 1e-12, "a cow sits at {cow:?}");
    assert_eq!(cart_seat_offset(Species::Cow, 0.7), PASSENGER_Y - 0.7);
    // A zombie villager is no `Villager`.
    assert_eq!(cart_attachment(Species::ZombieVillager), PASSENGER_Y);
}

/// `AbstractMinecart.getDismountLocationForPassenger` starts the search from
/// `getMotionDirection`, and `DismountHelper.offsetsForDirection` tries the
/// right of that direction first.
#[test]
fn an_eastbound_cart_sets_its_rider_down_on_its_south_side() {
    // `OldMinecartBehavior.tick`'s yaw of a cart moving east, flipped or not;
    // its `getMotionDirection` turns that facing clockwise, to east.
    for (yaw, flipped) in [(180.0, false), (0.0, true)] {
        let face = cart_motion_face(yaw, flipped, false);
        assert_eq!(
            dismount_offsets(face)[0],
            (0, 1),
            "yaw {yaw}, flipped {flipped}"
        );
    }
    // `MinecartBehavior.getMotionDirection` under the experiment is the
    // facing itself, so the first candidate stays east of a north-facing cart.
    let face = cart_motion_face(180.0, false, true);
    assert_eq!(dismount_offsets(face)[0], (1, 0));
}
