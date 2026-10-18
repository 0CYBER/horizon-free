//! LocalPlayer.sendPosition and PositionMoveRotation.calculateAbsolute.
use horizon_core::geometry::Vec3;
use horizon_protocol::packets::play::{self, PlayerPosition, Serverbound as S, relative};

/// Only movement updates advance this baseline. A teleport acknowledgement is
/// separate and must not suppress the next tick's corrected position or look.
#[derive(Default)]
pub struct PositionSender {
    position: [f64; 3],
    rotation: (f32, f32),
    on_ground: bool,
    collision: bool,
    reminder: u32,
}

impl PositionSender {
    pub fn update(
        &mut self,
        position: [f64; 3],
        rotation: (f32, f32),
        on_ground: bool,
        collision: bool,
    ) -> Option<S> {
        self.reminder += 1;
        let d: [f64; 3] = std::array::from_fn(|i| position[i] - self.position[i]);
        let moved =
            d[0] * d[0] + d[1] * d[1] + d[2] * d[2] > 2.0e-4 * 2.0e-4 || self.reminder >= 20;
        let rotated = rotation.0 - self.rotation.0 != 0.0 || rotation.1 - self.rotation.1 != 0.0;
        let flags = u8::from(on_ground) | u8::from(collision) << 1;
        let [x, y, z] = position;
        let (yaw, pitch) = rotation;
        let packet = if moved && rotated {
            Some(S::MovePlayerPosRot(play::MovePlayerPosRot {
                x,
                y,
                z,
                yaw,
                pitch,
                flags,
            }))
        } else if moved {
            Some(S::MovePlayerPos(play::MovePlayerPos { x, y, z, flags }))
        } else if rotated {
            Some(S::MovePlayerRot(play::MovePlayerRot { yaw, pitch, flags }))
        } else if self.on_ground != on_ground || self.collision != collision {
            Some(S::MovePlayerStatus(play::MovePlayerStatus { flags }))
        } else {
            None
        };
        if moved {
            self.position = position;
            self.reminder = 0;
        }
        if rotated {
            self.rotation = rotation;
        }
        self.on_ground = on_ground;
        self.collision = collision;
        packet
    }
}

/// Rotate the old velocity before adding the relative delta components.
/// Angles deliberately use Java's double toRadians followed by a float cast.
pub fn teleport_velocity(
    mut velocity: Vec3,
    old_rotation: (f32, f32),
    new_rotation: (f32, f32),
    packet: &PlayerPosition,
) -> Vec3 {
    if packet.relatives & relative::ROTATE_DELTA != 0 {
        let pitch = f64::from(old_rotation.1 - new_rotation.1).to_radians() as f32;
        let yaw = f64::from(old_rotation.0 - new_rotation.0).to_radians() as f32;
        velocity = velocity.rotate_x(pitch).rotate_y(yaw);
    }
    let old = [velocity.x, velocity.y, velocity.z];
    let flags = [relative::DELTA_X, relative::DELTA_Y, relative::DELTA_Z];
    let result: [f64; 3] = std::array::from_fn(|i| {
        if packet.relatives & flags[i] != 0 {
            old[i] + packet.delta[i]
        } else {
            packet.delta[i]
        }
    });
    Vec3::new(result[0], result[1], result[2])
}
