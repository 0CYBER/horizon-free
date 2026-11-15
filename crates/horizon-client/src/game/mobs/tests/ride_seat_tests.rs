//! `/ride` (`RideCommand.mount`): where a rider sits on its mount
//! (`Entity.getPassengerRidingPosition`, the vehicle type's own
//! `passengerAttachments`), a mob as a minecart's passenger, and the entity
//! vanilla lets nobody ride.
use super::tests::server_game;
use super::*;
use crate::game::commands::{OtherEntity, Target};
use crate::game::minecarts::{CartKind, Minecart};
use horizon_client::motion::PlayerMotion;

/// `EntityTypes`' `passengerAttachments` of 26.3: the rider's attachment point
/// above the vehicle's feet, and how far forward it sits. A type that lists
/// heights (`passengerAttachments(2.0125F)`) stores the float widened; one
/// that lists points (`new Vec3(0.0, 1.37, -0.3)`) stores those doubles.
const SEATS: [(&str, f64, f64); 8] = [
    ("minecraft:zombie", 2.0125_f32 as f64, 0.0),
    ("minecraft:spider", 0.765_f32 as f64, 0.0),
    ("minecraft:chicken", 0.7, -0.1),
    ("minecraft:ravager", 2.2625, -0.0625),
    ("minecraft:pig", 0.86875_f32 as f64, 0.0),
    ("minecraft:horse", 1.44375_f32 as f64, 0.0),
    ("minecraft:llama", 1.37, -0.3),
    ("minecraft:cow", 1.36875_f32 as f64, 0.0),
];

/// A mob of `kind` with id `id` standing at `at` facing south, alone in `game`.
fn mob_at(game: &mut Game<'_>, kind: &str, id: u32, at: [f64; 3]) {
    let kind = game.res.mob_types().get(kind).unwrap().clone();
    let m = horizon_client::mobs::Mob::new(id, kind, Vec3::new(at[0], at[1], at[2]), 0.0);
    game.mobs.clear();
    game.mobs.push(m);
}

/// A rideable minecart with id `id` at `at`.
fn cart_at(game: &mut Game<'_>, id: u32, at: [f64; 3]) {
    game.carts.clear();
    game.carts.push(Minecart::new(
        id,
        CartKind::Rideable,
        Vec3::new(at[0], at[1], at[2]),
    ));
}

/// `Entity.positionRider`: the player sits at the vehicle type's own passenger
/// attachment, turned with the vehicle and less the player's own 0.6 vehicle
/// attachment (`Avatar.DEFAULT_VEHICLE_ATTACHMENT`). A zombie's point is
/// 2.0125 above its feet, a spider's 0.765, a chicken's (0, 0.7, -0.1).
#[test]
fn a_rider_sits_at_the_vehicle_types_own_passenger_attachment() {
    let mut resources = None;
    let Some(mut game) = server_game(&mut resources) else {
        return;
    };
    let at = [1.5, 64.0, -2.5];
    game.player = Some(
        PlayerMotion::new(game.motion, Vec3::new(at[0], at[1], at[2])).expect("a player to ride"),
    );
    for (kind, y, z) in SEATS {
        mob_at(&mut game, kind, 1, at);
        game.riding = Some(1);
        game.position_rider();
        let p = game.player.as_ref().expect("a player").position;
        let (want_y, want_z) = (at[1] + y - 0.6, at[2] + z);
        assert_eq!(
            p.y, want_y,
            "{kind}: sits at {} but the type's attachment puts it at {want_y}",
            p.y
        );
        assert_eq!(
            p.z, want_z,
            "{kind}: sits at z {} but the type's attachment puts it at {want_z}",
            p.z
        );
    }
}

/// `/ride <mob> mount <minecart>`: `startRiding(vehicle, true, true)` forces
/// any mob onto a cart, which carries it on its 0.1875 passenger point less the
/// mob's own riding offset (a zombie hangs 0.7 below).
#[test]
fn a_mob_rides_a_minecart() {
    let mut resources = None;
    let Some(mut game) = server_game(&mut resources) else {
        return;
    };
    let at = [0.5, 64.0625, 0.5];
    mob_at(&mut game, "minecraft:zombie", 1, [0.5, 64.0, 0.5]);
    cart_at(&mut game, 7, at);
    game.ride_mount(Target::Mob(1), Target::Cart(7))
        .expect("a minecart takes a mob as its passenger");
    assert_eq!(
        game.mobs[0].extra.mount,
        Some(7),
        "the zombie rides the cart"
    );
    game.sync_riders();
    let p = game.mobs[0].position;
    let want = at[1] + 0.1875 - f64::from(0.7_f32);
    assert_eq!(
        p.y, want,
        "the zombie sits at {} but the cart's seat is {want}",
        p.y
    );
    assert_eq!(
        (p.x, p.z),
        (at[0], at[2]),
        "the cart's attachment has no x or z"
    );
}

/// `Marker.couldAcceptPassenger` is false, so `/ride`'s forced
/// `startRiding(vehicle, true, true)` fails there with the generic mount
/// failure; a display, which vanilla's `Entity` does let ride, still takes the
/// player on its zero-height passenger point.
#[test]
fn a_marker_takes_no_rider() {
    let mut resources = None;
    let Some(mut game) = server_game(&mut resources) else {
        return;
    };
    for (id, kind) in [(9, "minecraft:marker"), (10, "minecraft:item_display")] {
        game.others.push(OtherEntity {
            id,
            kind: kind.into(),
            position: Vec3::new(0.5, 64.0, 0.5),
            yaw: 0.0,
            pitch: 0.0,
            item_transformation: None,
            interaction: None,
            knot_check: None,
            eye: None,
            attachment_lifetime: Default::default(),
            join: Default::default(),
        });
    }
    let failed = game
        .ride_mount(Target::Player, Target::Other(9))
        .expect_err("a marker takes no rider");
    assert_eq!(
        failed.key, "commands.ride.mount.failure.generic",
        "vanilla's `ERROR_MOUNT_FAILED`"
    );
    assert_eq!(game.riding, None, "the player stays off the marker");
    game.player =
        Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).expect("a player to ride"));
    game.ride_mount(Target::Player, Target::Other(10))
        .expect("a display takes a rider");
    assert_eq!(game.riding, Some(10));
    let p = game.player.as_ref().expect("a player").position;
    assert_eq!(
        p.y, 63.4,
        "the display's seat is 0.6 below its feet: {}",
        p.y
    );
}
