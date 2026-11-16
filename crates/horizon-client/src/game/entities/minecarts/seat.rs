//! `AbstractMinecart`'s rider seat and the side it sets a rider down on:
//! `getPassengerAttachmentPoint` and `getDismountLocationForPassenger`.
use super::*;

/// AbstractMinecart passenger attachment Y (EntityAttachments).
pub(super) const PASSENGER_Y: f64 = 0.1875;

/// `AbstractMinecart.getPassengerAttachmentPoint`: a villager and a wandering
/// trader get `LOWERED_PASSENGER_ATTACHMENT`, the cart's own y.
pub(super) fn cart_attachment(s: Species) -> f64 {
    match s {
        Species::Villager | Species::WanderingTrader => 0.0,
        _ => PASSENGER_Y,
    }
}

/// `Entity.positionRider`'s offset on an `AbstractMinecart`: the cart's
/// passenger attachment less the rider's own `vehicle_attachment` (the cart's
/// box is 0.98 wide).
pub(super) fn cart_seat_offset(s: Species, vehicle_attachment: f64) -> f64 {
    cart_attachment(s) - vehicle_attachment
}

/// `AbstractMinecart.getMotionDirection` as a `from2DDataValue` value (south,
/// west, north, east): `OldMinecartBehavior` turns the cart's facing clockwise,
/// and the opposite facing's clockwise when the cart is flipped; the
/// experiment keeps the facing itself (`MinecartBehavior`).
pub(super) fn cart_motion_face(yaw: f32, flipped: bool, improvements: bool) -> i32 {
    let facing = (f64::from(yaw) / 90.0 + 0.5).floor() as i32;
    if improvements {
        facing
    } else {
        facing + if flipped { 3 } else { 1 }
    }
}

/// `DismountHelper.offsetsForDirection` for a `from2DDataValue` face: right,
/// left, back right, back left, forward right, forward left, back, forward.
pub(super) fn dismount_offsets(face: i32) -> [(i32, i32); 8] {
    let (fx, fz) = [(0, 1), (-1, 0), (0, -1), (1, 0)][(face & 3) as usize];
    // `right` is `Direction.getClockWise`.
    let (rx, rz) = (-fz, fx);
    [
        (rx, rz),
        (-rx, -rz),
        (rx - fx, rz - fz),
        (-rx - fx, -rz - fz),
        (fx + rx, fz + rz),
        (fx - rx, fz - rz),
        (-fx, -fz),
        (fx, fz),
    ]
}
