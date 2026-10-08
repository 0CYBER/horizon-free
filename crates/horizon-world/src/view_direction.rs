//! Native float-table view direction and horizontal blocking angle arithmetic.
use horizon_core::geometry::{Vec3, trig};
pub fn view_vector(pitch: f32, yaw: f32) -> Vec3 {
    let pitch = pitch * trig::DEG_TO_RAD;
    let yaw = -yaw * trig::DEG_TO_RAD;
    let cy = trig::cos(f64::from(yaw));
    let sy = trig::sin(f64::from(yaw));
    let cp = trig::cos(f64::from(pitch));
    let sp = trig::sin(f64::from(pitch));
    Vec3::new(f64::from(sy * cp), f64::from(-sp), f64::from(cy * cp))
}
/// Java Math.acos comparisons use the pinned pure-Rust implementation. The
/// platform C-library acos is not bit-identical to the reference JVM.
pub fn acos(value: f64) -> f64 {
    libm::acos(value)
}
pub fn blocking_angle(source: Vec3, position: Vec3, head_yaw: f32) -> f64 {
    let view = view_vector(0.0, head_yaw);
    let delta = source.subtract(position);
    let horizontal = Vec3::new(delta.x, 0.0, delta.z).normalize();
    acos(horizontal.dot(view))
}
/// Native no-source-position fallback widens the float PI constant to double.
pub const BLOCKING_FALLBACK_ANGLE: f64 = std::f32::consts::PI as f64;
