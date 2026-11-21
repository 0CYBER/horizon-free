//! A mod's camera roll (`native_camera_roll`): what faces the camera turns with the picture.
use super::{camera_rotation, rolled_camera};
use horizon_client::camera::{CameraType, view_matrix, view_matrix_rolled, view_roll};

/// The camera a billboard faces, inverted, is the view the world is drawn with: the roll in
/// front of the view (`view_roll`) and `view_matrix_rolled`, NeoForge's `Camera.setRotation`.
#[test]
fn billboards_roll_with_the_view() {
    for (yaw, pitch, roll) in [
        (0.0, 0.0, 90.0),
        (37.0, -21.0, 22.5),
        (-137.25, 61.5, -45.0),
        (210.0, 89.0, 180.0),
    ] {
        let facing = rolled_camera(camera_rotation(CameraType::FirstPerson, yaw, pitch), roll);
        let from_billboards = glam::Mat4::from_quat(facing.conjugate());
        for view in [
            view_roll(roll) * view_matrix(yaw, pitch),
            view_matrix_rolled(yaw, pitch, roll),
        ] {
            assert!(
                (from_billboards.to_cols_array().iter())
                    .zip(view.to_cols_array())
                    .all(|(a, b)| (a - b).abs() < 1e-5),
                "yaw {yaw} pitch {pitch} roll {roll}: {from_billboards} vs {view}"
            );
        }
    }
}

/// No roll leaves the camera's rotation as it was, to the bit.
#[test]
fn no_roll_keeps_the_camera_rotation() {
    let rotation = camera_rotation(CameraType::ThirdPersonFront, 12.5, -7.75);
    assert_eq!(rolled_camera(rotation, 0.0), rotation);
}
