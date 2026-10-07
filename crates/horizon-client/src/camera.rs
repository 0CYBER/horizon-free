//! Free inspection camera, deliberately separate from future player simulation.
use glam::{DVec3, Mat4, Vec3};
use std::collections::HashSet;
use winit::keyboard::KeyCode;
pub struct Camera {
    pub position: DVec3,
    pub yaw: f64,
    pub pitch: f64,
}
impl Default for Camera {
    fn default() -> Self {
        Self {
            position: DVec3::new(25.0, 18.0, 25.0),
            yaw: -135_f64.to_radians(),
            pitch: -22_f64.to_radians(),
        }
    }
}
impl Camera {
    /// A standing-sized inspection volume, not a simulated player entity.
    pub fn collision_bounds(&self) -> horizon_core::geometry::Aabb {
        use horizon_core::geometry::{Aabb, Vec3};
        let half_width = f64::from(0.6_f32 / 2.0);
        let feet = self.position.y - f64::from(1.62_f32);
        Aabb::new(
            Vec3::new(
                self.position.x - half_width,
                feet,
                self.position.z - half_width,
            ),
            Vec3::new(
                self.position.x + half_width,
                feet + f64::from(1.8_f32),
                self.position.z + half_width,
            ),
        )
    }
    pub fn look(&mut self, x: f64, y: f64) {
        self.yaw -= x * 0.002;
        self.pitch = (self.pitch - y * 0.002).clamp(-1.55, 1.55);
    }
    pub fn update(&mut self, keys: &HashSet<KeyCode>, seconds: f64) {
        self.turn(keys, seconds);
        let held = |key| f64::from(u8::from(keys.contains(&key)));
        let forward = DVec3::new(self.yaw.sin(), 0.0, self.yaw.cos());
        let right = DVec3::new(-self.yaw.cos(), 0.0, self.yaw.sin());
        let movement = forward * (held(KeyCode::KeyW) - held(KeyCode::KeyS))
            + right * (held(KeyCode::KeyD) - held(KeyCode::KeyA))
            + DVec3::Y
                * (held(KeyCode::Space) - held(KeyCode::ShiftLeft) - held(KeyCode::ShiftRight));
        if movement.length_squared() > 0.0 {
            self.position += movement.normalize()
                * seconds
                * if keys.contains(&KeyCode::ControlLeft) || keys.contains(&KeyCode::ControlRight) {
                    40.0
                } else {
                    12.0
                };
        }
    }
    pub fn turn(&mut self, keys: &HashSet<KeyCode>, seconds: f64) {
        let held = |key| f64::from(u8::from(keys.contains(&key)));
        self.yaw += (held(KeyCode::ArrowLeft) - held(KeyCode::ArrowRight)) * seconds * 1.5;
        self.pitch = (self.pitch
            + (held(KeyCode::ArrowUp) - held(KeyCode::ArrowDown)) * seconds * 1.5)
            .clamp(-1.55, 1.55);
    }
    pub fn matrix(&self, aspect: f32) -> Mat4 {
        let forward = Vec3::new(
            (self.yaw.sin() * self.pitch.cos()) as f32,
            self.pitch.sin() as f32,
            (self.yaw.cos() * self.pitch.cos()) as f32,
        );
        Mat4::perspective_rh(70_f32.to_radians(), aspect, 0.05, 1024.0)
            * Mat4::look_to_rh(self.position.as_vec3(), forward, Vec3::Y)
    }
}
