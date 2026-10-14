//! `EndCrystalRenderer` and `EndCrystalModel.setupAnim`: the crystal's bob
//! and the poses of its base, glass and cube.
use crate::mobs::{Part, anim::Poses};
use horizon_core::geometry::trig::Mth;

/// `EndCrystalRenderer.getY`: the crystal's bob below its resting height.
pub fn y(age: f32) -> f32 {
    let hh = (age * 0.2).msin() / 2.0 + 0.5;
    (hh * hh + hh) * 0.4 - 1.4
}

/// `EndCrystalModel.setupAnim`: the base shown or not, the glass raised by
/// half the bob (in model pixels) and the three cubes turned with
/// `ModelPart.rotateBy`, which stores the composed rotation as ZYX angles.
pub fn poses(model: &Part, age: f32, show_bottom: bool) -> Poses {
    let mut poses = Poses::new(model);
    poses.visible("base", show_bottom);
    let speed = age * 3.0;
    let sin45 = std::f32::consts::FRAC_PI_4.sin();
    let tilt = glam::Quat::from_axis_angle(
        glam::Vec3::new(sin45, 0.0, sin45),
        std::f32::consts::FRAC_PI_3,
    );
    let spin = glam::Quat::from_rotation_y(speed.to_radians());
    poses.get("outer_glass")[1] += y(age) * 16.0 / 2.0;
    for (part, rotation) in [
        ("outer_glass", spin * tilt),
        ("inner_glass", tilt * spin),
        ("cube", tilt * spin),
    ] {
        let (z, y, x) = rotation.to_euler(glam::EulerRot::ZYX);
        let pose = poses.get(part);
        pose[3] = x;
        pose[4] = y;
        pose[5] = z;
    }
    poses
}
