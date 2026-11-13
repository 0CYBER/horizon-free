//! The camera's view frustum, as camera-relative clip planes.

use glam::{Vec3, Vec4};

/// Camera-relative clip planes (`Frustum`).
#[derive(Clone, Copy, Debug)]
pub struct Frustum {
    pub(super) planes: [Vec4; 6],
}
impl Frustum {
    /// Whether the camera-relative box `min..max` is at least partly inside.
    pub fn intersects(&self, min: Vec3, max: Vec3) -> bool {
        !self.planes.iter().any(|p| {
            let sign = p.truncate().cmpge(Vec3::ZERO);
            let v = Vec3::select(sign, max, min);
            p.truncate().dot(v) + p.w < 0.0
        })
    }
    /// Whether a camera-relative sphere is at least partly inside. Frustum
    /// planes are normalized, so each test needs only one dot product and a
    /// radius comparison.
    pub fn intersects_sphere(&self, center: Vec3, radius: f32) -> bool {
        if !center.is_finite() || !radius.is_finite() || radius < 0.0 {
            return true;
        }
        !self
            .planes
            .iter()
            .any(|plane| plane.truncate().dot(center) + plane.w < -radius)
    }
    /// Depth clip planes alone: X/Y projection jitter cannot change this test.
    /// Retaining side-of-screen geometry makes this safe before TAA selects a phase.
    pub fn intersects_depth(&self, min: Vec3, max: Vec3) -> bool {
        if !min.is_finite() || !max.is_finite() || !min.cmple(max).all() {
            return true;
        }
        !self.planes[4..].iter().any(|p| {
            let sign = p.truncate().cmpge(Vec3::ZERO);
            let v = Vec3::select(sign, max, min);
            p.truncate().dot(v) + p.w < 0.0
        })
    }
    /// Whether the camera-relative box `min..max` is entirely inside. Then
    /// `intersects` accepts every box within it: each plane's distance only
    /// grows from this box's nearest corner to the inner box's farthest, and
    /// rounding keeps that order.
    pub fn contains(&self, min: Vec3, max: Vec3) -> bool {
        self.planes.iter().all(|p| {
            let sign = p.truncate().cmpge(Vec3::ZERO);
            let v = Vec3::select(sign, min, max);
            p.truncate().dot(v) + p.w >= 0.0
        })
    }
    /// Whether the camera-relative `point` is on or inside every plane (vanilla's
    /// `Frustum.pointInFrustum`). A point that is not finite is kept: a test that cannot
    /// tell must not hide the particle.
    pub fn contains_point(&self, point: Vec3) -> bool {
        if !point.is_finite() {
            return true;
        }
        self.planes
            .iter()
            .all(|plane| plane.truncate().dot(point) + plane.w >= 0.0)
    }
    /// The same frustum seen from a camera moved `blocks` back along the view direction
    /// (vanilla's `Frustum.offset`): the sides open up near the camera and the near plane
    /// moves behind it, so what sits just behind or beside the camera stays inside.
    ///
    /// The view direction is the normal of the depth planes (the two are opposite), signed
    /// towards where the side planes point. A frustum that has no direction to pull along
    /// comes back unchanged: a box with no depth planes (the shadow pass's), and a box whose
    /// sides are parallel to the view direction (an orthographic projection), where the side
    /// normals cancel and say nothing about which way is forward.
    pub fn pulled_back(&self, blocks: f32) -> Frustum {
        let sides: Vec3 = self.planes[..4].iter().map(|p| p.truncate()).sum();
        let depth = self.planes[4].truncate();
        let along = depth.dot(sides);
        // A perspective frustum's four side normals each lean forward, so this is well above 1.
        if along.abs() < 1.0e-3 {
            return *self;
        }
        let toward_sides = if along > 0.0 { depth } else { -depth };
        let axis = toward_sides.normalize_or_zero();
        let mut planes = self.planes;
        for plane in &mut planes {
            plane.w += blocks * plane.truncate().dot(axis);
        }
        Frustum { planes }
    }
    /// The frustum of a camera-relative `view_proj`, for tests outside this module.
    #[cfg(test)]
    pub(crate) fn from_view_projection(view_proj: glam::Mat4) -> Frustum {
        Frustum {
            planes: super::frustum(view_proj),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::{Frustum, Vec3};
    use crate::gfx::world_projection;
    use glam::Mat4;

    const FOV: f32 = 70.0;
    const ASPECT: f32 = 16.0 / 9.0;
    const FAR: f32 = 512.0;
    const EPS: f32 = 0.01;

    fn looking_down_minus_z() -> Frustum {
        Frustum::from_view_projection(world_projection(FOV, ASPECT, FAR))
    }

    fn tangents() -> (f32, f32) {
        let up = (FOV / 2.0).to_radians().tan();
        (up * ASPECT, up)
    }

    /// Each of the six planes of the unmoved frustum, a point just inside and one just outside,
    /// from the field of view worked out by hand.
    #[test]
    fn a_point_is_inside_only_within_all_six_planes() {
        let view = looking_down_minus_z();
        let (tan_x, tan_y) = tangents();
        let depth = 10.0;
        let inside = [
            Vec3::new(0.0, 0.0, -depth),
            Vec3::new(depth * tan_x - EPS, 0.0, -depth),
            Vec3::new(-(depth * tan_x - EPS), 0.0, -depth),
            Vec3::new(0.0, depth * tan_y - EPS, -depth),
            Vec3::new(0.0, -(depth * tan_y - EPS), -depth),
            Vec3::new(0.0, 0.0, -0.06),
            Vec3::new(0.0, 0.0, -(FAR - 0.1)),
        ];
        let outside = [
            Vec3::new(depth * tan_x + EPS, 0.0, -depth),
            Vec3::new(-(depth * tan_x + EPS), 0.0, -depth),
            Vec3::new(0.0, depth * tan_y + EPS, -depth),
            Vec3::new(0.0, -(depth * tan_y + EPS), -depth),
            Vec3::new(0.0, 0.0, -0.04),
            Vec3::new(0.0, 0.0, -(FAR + 0.1)),
            Vec3::new(0.0, 0.0, 1.0),
            Vec3::new(0.0, 0.0, 0.0),
        ];
        for p in inside {
            assert!(view.contains_point(p), "{p} should be inside");
        }
        for p in outside {
            assert!(!view.contains_point(p), "{p} should be outside");
        }
        assert!(view.contains_point(Vec3::NAN), "an unknown point is kept");
    }

    /// Camera 3 blocks back: its apex is at z = 3, so the sides open by 3 * tan, the near
    /// plane is at z = 3 - 0.05 and the far plane at z = 3 - FAR.
    #[test]
    fn pulling_the_camera_back_moves_every_plane() {
        let (tan_x, tan_y) = tangents();
        let back = looking_down_minus_z().pulled_back(3.0);
        let inside = [
            Vec3::new(0.0, 0.0, 2.9),
            Vec3::new(3.0 * tan_x - EPS, 0.0, 0.0),
            Vec3::new(-(3.0 * tan_x - EPS), 0.0, 0.0),
            Vec3::new(0.0, 3.0 * tan_y - EPS, 0.0),
            Vec3::new(0.0, -(3.0 * tan_y - EPS), 0.0),
            Vec3::new(0.0, 0.0, 3.0 - FAR + 0.1),
        ];
        let outside = [
            Vec3::new(0.0, 0.0, 3.0),
            Vec3::new(3.0 * tan_x + EPS, 0.0, 0.0),
            Vec3::new(-(3.0 * tan_x + EPS), 0.0, 0.0),
            Vec3::new(0.0, 3.0 * tan_y + EPS, 0.0),
            Vec3::new(0.0, -(3.0 * tan_y + EPS), 0.0),
            Vec3::new(0.0, 0.0, 3.0 - FAR - 0.1),
        ];
        for p in inside {
            assert!(back.contains_point(p), "{p} should be inside");
        }
        for p in outside {
            assert!(!back.contains_point(p), "{p} should be outside");
        }
        // The unmoved frustum does not have the point that only the offset one has.
        assert!(!looking_down_minus_z().contains_point(Vec3::new(0.0, 0.0, 2.9)));
        // An offset of nothing changes no verdict.
        let same = looking_down_minus_z();
        let unmoved = same.pulled_back(0.0);
        for p in [
            Vec3::new(1.0, 2.0, -3.0),
            Vec3::new(0.0, 0.0, 1.0),
            Vec3::new(40.0, 0.0, -5.0),
        ] {
            assert_eq!(same.contains_point(p), unmoved.contains_point(p));
        }
    }

    /// The offset follows the camera's own view direction, whatever way it looks.
    #[test]
    fn the_offset_is_along_the_view_direction() {
        for (yaw, pitch) in [(0.3_f32, 0.0_f32), (2.0, -0.5), (-1.2, 0.9), (3.0, 1.2)] {
            let pose = Mat4::from_rotation_x(pitch) * Mat4::from_rotation_y(yaw);
            let view = Frustum::from_view_projection(world_projection(FOV, ASPECT, FAR) * pose);
            let forward = pose.inverse().transform_vector3(-Vec3::Z);
            let back = view.pulled_back(3.0);
            assert!(
                back.contains_point(forward * -2.9),
                "{yaw} {pitch}: 2.9 behind"
            );
            assert!(
                !back.contains_point(forward * -3.0),
                "{yaw} {pitch}: 3 behind"
            );
            assert!(
                !view.contains_point(forward * -2.9),
                "{yaw} {pitch}: unmoved"
            );
            assert!(back.contains_point(forward * 20.0) && view.contains_point(forward * 20.0));
        }
    }

    /// An orthographic box has no apex to pull back from: its side normals cancel, so there is
    /// no direction, and the frustum must come back as it was.
    #[test]
    fn an_orthographic_frustum_stays_where_it_is() {
        let ortho =
            Frustum::from_view_projection(Mat4::orthographic_rh(-8.0, 8.0, -4.5, 4.5, 100.0, 0.05));
        let moved = ortho.pulled_back(3.0);
        for p in [
            Vec3::new(0.0, 0.0, 2.0),
            Vec3::new(0.0, 0.0, -9.0),
            Vec3::new(7.9, 4.4, -9.0),
            Vec3::new(8.1, 0.0, -9.0),
            Vec3::new(0.0, 0.0, -101.0),
        ] {
            assert_eq!(ortho.contains_point(p), moved.contains_point(p), "{p}");
        }
    }

    /// The shadow pass's box has no depth planes, so no direction: it must not move.
    #[test]
    fn a_frustum_without_depth_planes_stays_where_it_is() {
        let mut flat = looking_down_minus_z();
        flat.planes[4] = glam::Vec4::W;
        flat.planes[5] = glam::Vec4::W;
        let moved = flat.pulled_back(3.0);
        for p in [
            Vec3::new(0.0, 0.0, 2.0),
            Vec3::new(0.0, 0.0, -9.0),
            Vec3::new(80.0, 0.0, -9.0),
        ] {
            assert_eq!(flat.contains_point(p), moved.contains_point(p));
        }
    }
}
