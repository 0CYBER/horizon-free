//! CPU-side TFR math: camera cut tests, reprojection matrices and sample validity.
//!
//! Conventions (aligned with Horizon renderer, adapted from `TFR_DESCRIPTION.md`):
//! - Camera-relative rotation-only view-projection matrices are used for reprojection:
//!   `vp = projection * pose * rotation` (no absolute camera translation baked in), as in
//!   `world_post::reprojection`. Camera world positions are supplied separately.
//! - World space is right-handed, Y up. Matrices are column-major (`glam`), matching WGSL.
//! - Clip is `m * vec4(x,y,z,1)`; NDC is `clip.xyz / clip.w`.
//! - UV is normalized `[0, 1]` with origin at the texture top-left:
//!   `uv = ndc.xy * vec2(0.5, -0.5) + 0.5`, `ndc = uv * vec2(2, -2) + vec2(-1, 1)`.
//! - Horizon uses reversed-Z: normalized depth/WebGPU clip depth is `[0, 1]` where
//!   `1` is near and `0` is far/sky (`camera.rs`, `world_post.wgsl`, `post_chain.rs`).
//! - Far/sky is detected near depth `0` and uses the sky reprojection matrix
//!   (no translation parallax): `sky = current_vp * reference_vp_inv`.
//! - Geometry reprojection keeps world points fixed:
//!   `geometry = current_vp * T(reference_camera - current_camera) * reference_vp_inv`.
//! - A reprojected sample is invalid when `w <= W_EPSILON` or its projection/depth is
//!   non-finite. The target UV may leave `[0, 1]`: the forward vector is still kept so a
//!   compose pass can walk from a destination sample back to a source inside the reference.

use glam::{DMat4, DVec3, Mat4, Vec2, Vec3};

/// Small positive threshold: `w <= W_EPSILON` is invalid.
pub const W_EPSILON: f32 = 1.0e-6;
/// Tolerance at screen/depth bounds for stability.
pub const BOUNDS_EPSILON: f32 = 1.0e-4;
/// Depth values at or below this are treated as far/sky in Horizon's reversed-Z.
pub const FAR_DEPTH: f32 = 1.0e-6;

/// Meaningful-motion threshold in world units (below cut range), aligned with core intent.
pub const MOTION_DISTANCE_EPS: f64 = 0.001;
/// Meaningful-motion threshold in degrees (below cut range), aligned with core intent.
pub const MOTION_ANGLE_EPS_DEG: f64 = 0.01;

/// Thresholds below cut limits that gate motion-field recomputation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MotionThresholds {
    /// World units of camera translation.
    pub distance: f64,
    /// Degrees of camera rotation.
    pub angle_degrees: f64,
}

impl Default for MotionThresholds {
    fn default() -> Self {
        Self {
            distance: MOTION_DISTANCE_EPS,
            angle_degrees: MOTION_ANGLE_EPS_DEG,
        }
    }
}

/// Camera state for one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraState {
    /// World-space camera position.
    pub position: DVec3,
    /// Yaw in degrees, Horizon convention.
    pub yaw_degrees: f32,
    /// Pitch in degrees, Horizon convention.
    pub pitch_degrees: f32,
}

/// Normalizes a yaw delta to `(-180, 180]` degrees.
pub fn wrap_yaw_degrees(delta: f32) -> f32 {
    let mut d = delta % 360.0;
    if d > 180.0 {
        d -= 360.0;
    } else if d <= -180.0 {
        d += 360.0;
    }
    d
}

/// Clamps pitch to Horizon's valid range.
pub fn clamp_pitch_degrees(pitch: f32) -> f32 {
    pitch.clamp(-90.0, 90.0)
}

/// Euclidean norm of wrapped yaw and clamped-pitch deltas.
pub fn orientation_delta_degrees(a: CameraState, b: CameraState) -> f32 {
    let dyaw = wrap_yaw_degrees(a.yaw_degrees - b.yaw_degrees);
    let dpitch = clamp_pitch_degrees(a.pitch_degrees) - clamp_pitch_degrees(b.pitch_degrees);
    (dyaw * dyaw + dpitch * dpitch).sqrt()
}

/// Squared world-space camera translation.
pub fn translation_distance_sq(a: CameraState, b: CameraState) -> f64 {
    a.position.distance_squared(b.position)
}

/// True when reconstruction must be cut due to excessive camera distance.
pub fn distance_cut(current: CameraState, reference: CameraState, threshold: f64) -> bool {
    translation_distance_sq(current, reference) > threshold * threshold
}

/// True when reconstruction must be cut due to excessive camera rotation.
pub fn angle_cut(current: CameraState, reference: CameraState, threshold_degrees: f32) -> bool {
    orientation_delta_degrees(current, reference) > threshold_degrees
}

/// True when the camera moved enough to warrant recomputing the motion field.
pub fn has_meaningful_motion(
    current: CameraState,
    reference: CameraState,
    thresholds: MotionThresholds,
) -> bool {
    translation_distance_sq(current, reference) > thresholds.distance * thresholds.distance
        || f64::from(orientation_delta_degrees(current, reference)) > thresholds.angle_degrees
}

/// UV `[0,1]` (top-left origin) to NDC xy `[-1,1]`.
pub fn uv_to_ndc(uv: Vec2) -> Vec2 {
    Vec2::new(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0)
}

/// NDC xy `[-1,1]` to UV `[0,1]` (top-left origin).
pub fn ndc_to_uv(ndc: Vec2) -> Vec2 {
    Vec2::new(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5)
}

/// Normalized depth is WebGPU clip depth `[0,1]` in Horizon's reversed-Z.
pub fn depth_to_clip_z(depth: f32) -> f32 {
    depth
}

/// WebGPU clip depth `[0,1]` to normalized depth.
pub fn clip_z_to_depth(clip_z: f32) -> f32 {
    clip_z
}

/// Reprojection matrices for a synthetic frame.
///
/// `geometry` maps reference clip to current clip with camera translation applied.
/// `sky` maps reference clip to current clip without translation parallax.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReprojectionMatrices {
    pub geometry: Mat4,
    pub sky: Mat4,
}

/// Builds forward reprojection matrices (reference clip -> current clip).
///
/// `reference_vp` and `current_vp` must be rotation-only camera-relative
/// view-projection matrices (`projection * pose * rotation`), not matrices with
/// absolute camera translation baked in.
pub fn reprojection_matrices(
    reference_vp: DMat4,
    current_vp: DMat4,
    reference_camera: DVec3,
    current_camera: DVec3,
) -> ReprojectionMatrices {
    let inv = reference_vp.inverse();
    let geometry =
        (current_vp * DMat4::from_translation(reference_camera - current_camera) * inv).as_mat4();
    let sky = (current_vp * inv).as_mat4();
    ReprojectionMatrices { geometry, sky }
}

/// Selects the geometry or sky reprojection matrix for a depth sample.
pub fn select_reprojection(matrices: ReprojectionMatrices, depth: f32) -> Mat4 {
    if depth <= FAR_DEPTH {
        matrices.sky
    } else {
        matrices.geometry
    }
}

/// Result of projecting one reference sample into the current frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReprojectedSample {
    /// Target UV in the current frame.
    pub uv: Vec2,
    /// Target normalized depth in the current frame.
    pub depth: f32,
    /// Screen-space forward motion from source UV to target UV.
    pub motion: Vec2,
    /// Homogeneous w after reprojection.
    pub w: f32,
    /// Whether the sample is valid for reconstruction.
    pub valid: bool,
}

/// Reprojects one reference sample `(source_uv, depth)` into the current frame.
pub fn reproject_sample(
    source_uv: Vec2,
    depth: f32,
    matrices: ReprojectionMatrices,
) -> ReprojectedSample {
    let matrix = select_reprojection(matrices, depth);
    let ndc_xy = uv_to_ndc(source_uv);
    let clip_z = depth_to_clip_z(depth);
    let clip = matrix * Vec3::new(ndc_xy.x, ndc_xy.y, clip_z).extend(1.0);
    let w = clip.w;
    if w <= W_EPSILON || !w.is_finite() {
        return ReprojectedSample {
            uv: Vec2::ZERO,
            depth: 0.0,
            motion: Vec2::ZERO,
            w,
            valid: false,
        };
    }
    let inv_w = 1.0 / w;
    let ndc = clip.truncate() * inv_w;
    let uv = ndc_to_uv(ndc.truncate());
    let target_depth = clip_z_to_depth(ndc.z);
    let motion = uv - source_uv;
    // A target outside the viewport still carries a usable forward vector: compose uses it to
    // step from a destination pixel back to a source that is inside the reference image.
    let valid = uv.x.is_finite()
        && uv.y.is_finite()
        && ndc.z.is_finite()
        && (-BOUNDS_EPSILON..=1.0 + BOUNDS_EPSILON).contains(&target_depth);
    ReprojectedSample {
        uv,
        depth: target_depth,
        motion,
        w,
        valid,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx_vec2(a: Vec2, b: Vec2, eps: f32) {
        let d = (a - b).length();
        assert!(d < eps, "{a:?} != {b:?} (|d|={d})");
    }

    #[test]
    fn yaw_wraps_and_pitch_clamps() {
        assert!((wrap_yaw_degrees(350.0 - 10.0) - (-20.0)).abs() < 1.0e-6);
        assert!((wrap_yaw_degrees(10.0 - 350.0) - 20.0).abs() < 1.0e-6);
        assert!((wrap_yaw_degrees(181.0) - (-179.0)).abs() < 1.0e-6);
        assert_eq!(clamp_pitch_degrees(95.0), 90.0);
        assert_eq!(clamp_pitch_degrees(-100.0), -90.0);
    }

    #[test]
    fn distance_cut_uses_squared_threshold() {
        let a = CameraState {
            position: DVec3::ZERO,
            yaw_degrees: 0.0,
            pitch_degrees: 0.0,
        };
        let below = CameraState {
            position: DVec3::new(3.0, 4.0, 0.0),
            ..a
        };
        let above = CameraState {
            position: DVec3::new(3.0, 4.01, 0.0),
            ..a
        };
        assert!(!distance_cut(below, a, 5.0));
        assert!(distance_cut(above, a, 5.0));
    }

    #[test]
    fn angle_cut_uses_wrapped_yaw_and_clamped_pitch() {
        let a = CameraState {
            position: DVec3::ZERO,
            yaw_degrees: 10.0,
            pitch_degrees: 89.0,
        };
        let wrapped = CameraState {
            yaw_degrees: 350.0,
            pitch_degrees: 89.0,
            ..a
        };
        let clamped = CameraState {
            yaw_degrees: 10.0,
            pitch_degrees: 95.0,
            ..a
        };
        assert!((orientation_delta_degrees(wrapped, a) - 20.0).abs() < 1.0e-5);
        assert!((orientation_delta_degrees(clamped, a) - 1.0).abs() < 1.0e-5);
        assert!(!angle_cut(
            CameraState {
                yaw_degrees: 13.0,
                pitch_degrees: 85.0,
                ..a
            },
            a,
            5.0
        ));
        assert!(angle_cut(
            CameraState {
                yaw_degrees: 13.0,
                pitch_degrees: 84.9,
                ..a
            },
            a,
            5.0
        ));
    }

    #[test]
    fn uv_ndc_depth_round_trip() {
        for uv in [
            Vec2::ZERO,
            Vec2::new(1.0, 0.0),
            Vec2::new(0.25, 0.75),
            Vec2::ONE,
        ] {
            approx_vec2(ndc_to_uv(uv_to_ndc(uv)), uv, 1.0e-6);
        }
        for depth in [0.0, 0.25, 0.5, 1.0] {
            let clip = depth_to_clip_z(depth);
            assert!((clip_z_to_depth(clip) - depth).abs() < 1.0e-6);
        }
    }

    /// Rotation-only camera-relative VP (projection * pose * rotation), Horizon reversed-Z.
    fn rotation_vp(yaw: f64, pitch: f64) -> DMat4 {
        let projection = DMat4::perspective_rh(70.0_f64.to_radians(), 16.0 / 9.0, 1024.0, 0.05);
        let view =
            DMat4::from_rotation_x(pitch.to_radians()) * DMat4::from_rotation_y(yaw.to_radians());
        projection * view
    }

    #[test]
    fn static_camera_geometry_reprojects_to_same_uv() {
        let camera = DVec3::new(10.0, 64.0, -3.0);
        let state = CameraState {
            position: camera,
            yaw_degrees: 20.0,
            pitch_degrees: -10.0,
        };
        let vp = rotation_vp(20.0, -10.0);
        let matrices = reprojection_matrices(vp, vp, state.position, state.position);
        let sample = reproject_sample(Vec2::new(0.37, 0.61), 0.42, matrices);
        assert!(sample.valid, "{sample:?}");
        approx_vec2(sample.uv, Vec2::new(0.37, 0.61), 1.0e-4);
        assert!(sample.motion.length() < 1.0e-4, "{sample:?}");
    }

    #[test]
    fn translated_camera_shifts_geometry_but_not_sky() {
        let reference = CameraState {
            position: DVec3::new(0.0, 64.0, 0.0),
            yaw_degrees: 0.0,
            pitch_degrees: 0.0,
        };
        let current = CameraState {
            position: DVec3::new(0.25, 64.0, 0.0),
            ..reference
        };
        let vp = rotation_vp(0.0, 0.0);
        let matrices = reprojection_matrices(vp, vp, reference.position, current.position);

        // A centered point 4 blocks ahead in camera-relative space.
        let point = DVec3::new(0.0, 0.0, -4.0);
        let ref_clip = vp * point.extend(1.0);
        let ref_ndc = ref_clip.truncate() / ref_clip.w;
        let source_uv = ndc_to_uv(Vec2::new(ref_ndc.x as f32, ref_ndc.y as f32));
        let depth = ref_ndc.z as f32; // reversed-Z clip depth [0,1]

        let geom = reproject_sample(source_uv, depth, matrices);
        let sky = reproject_sample(source_uv, FAR_DEPTH, matrices);
        assert!(geom.valid, "{geom:?}");
        assert!(sky.valid, "{sky:?}");

        // Expected current clip: same world point, camera moved +X => camera-relative x becomes -0.25.
        let expected_clip = vp * DVec3::new(-0.25, 0.0, -4.0).extend(1.0);
        let expected_ndc = expected_clip.truncate() / expected_clip.w;
        let expected_uv = ndc_to_uv(Vec2::new(expected_ndc.x as f32, expected_ndc.y as f32));
        approx_vec2(geom.uv, expected_uv, 2.0e-4);
        approx_vec2(sky.uv, source_uv, 2.0e-4);
        assert!(geom.motion.length() > 1.0e-4, "{geom:?}");
        assert!(sky.motion.length() < 1.0e-4, "{sky:?}");
    }

    #[test]
    fn sky_reprojects_rotation_without_translation_parallax() {
        let reference = CameraState {
            position: DVec3::new(0.0, 64.0, 0.0),
            yaw_degrees: 0.0,
            pitch_degrees: 0.0,
        };
        let current = CameraState {
            position: DVec3::new(5.0, 64.0, -3.0),
            yaw_degrees: 10.0,
            pitch_degrees: 0.0,
        };
        let ref_vp = rotation_vp(0.0, 0.0);
        let cur_vp = rotation_vp(10.0, 0.0);
        let matrices = reprojection_matrices(ref_vp, cur_vp, reference.position, current.position);

        // Expected values by hand, from the pinhole model alone: a view-space direction (x, y, z)
        // lands on NDC (f/aspect * x/-z, f * y/-z) with f = 1/tan(35 deg), and UV is
        // (0.5 + ndc.x/2, 0.5 - ndc.y/2). The reference view is the identity, so the sky sample
        // looking along (0.2, 0.1, -1) sits off-centre on both axes. A 10 degree yaw (glam's
        // rotation about +Y) turns it into (0.2c - s, 0.1, -0.2s - c); the camera's move adds
        // nothing to a direction.
        let f = 1.0 / 35.0_f64.to_radians().tan();
        let aspect = 16.0 / 9.0;
        let (s, c) = 10.0_f64.to_radians().sin_cos();
        let source_uv = Vec2::new(
            (0.5 + 0.5 * f / aspect * 0.2) as f32,
            (0.5 - 0.5 * f * 0.1) as f32,
        );
        let depth_w = 0.2 * s + c;
        let expected = Vec2::new(
            (0.5 + 0.5 * f / aspect * (0.2 * c - s) / depth_w) as f32,
            (0.5 - 0.5 * f * 0.1 / depth_w) as f32,
        );

        let sky = reproject_sample(source_uv, 0.0, matrices);
        assert!(sky.valid, "{sky:?}");
        approx_vec2(sky.uv, expected, 2.0e-4);
    }

    #[test]
    fn invalid_when_w_non_positive_and_valid_when_target_leaves_viewport() {
        let zero_w = reproject_sample(
            Vec2::new(0.5, 0.5),
            0.4,
            ReprojectionMatrices {
                geometry: Mat4::from_cols_array(&[
                    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0,
                ]),
                sky: Mat4::IDENTITY,
            },
        );
        assert!(!zero_w.valid);
        assert!(zero_w.w <= W_EPSILON);

        let vp = rotation_vp(0.0, 0.0);
        let matrices = reprojection_matrices(vp, vp, DVec3::ZERO, DVec3::ZERO);
        // An off-screen target still carries a forward vector compose can follow back to an
        // in-view source; it is valid as long as the projection and depth are finite.
        let oob = reproject_sample(Vec2::new(1.1, 0.5), 0.2, matrices);
        assert!(oob.valid, "{oob:?}");
        assert!(oob.uv.x > 1.0, "{oob:?}");
    }

    #[test]
    fn forward_motion_survives_when_target_leaves_viewport() {
        // Identity projection with a +0.2 NDC x shift: a source at 0.95 lands at 1.05.
        // Compose needs that forward vector to find the source for a trailing-edge pixel.
        let shifted = ReprojectionMatrices {
            geometry: Mat4::from_cols_array(&[
                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.2, 0.0, 0.0, 1.0,
            ]),
            sky: Mat4::IDENTITY,
        };
        let sample = reproject_sample(Vec2::new(0.95, 0.5), 0.4, shifted);
        assert!(sample.valid, "{sample:?}");
        assert!(sample.uv.x > 1.0, "{sample:?}");
        assert!((sample.motion.x - 0.1).abs() < 1.0e-4, "{sample:?}");
    }

    #[test]
    fn meaningful_motion_uses_core_aligned_epsilons() {
        let a = CameraState {
            position: DVec3::ZERO,
            yaw_degrees: 0.0,
            pitch_degrees: 0.0,
        };
        let below = CameraState {
            position: DVec3::new(0.0005, 0.0, 0.0),
            yaw_degrees: 0.005,
            pitch_degrees: 0.0,
        };
        let above_dist = CameraState {
            position: DVec3::new(0.002, 0.0, 0.0),
            ..a
        };
        let above_angle = CameraState {
            yaw_degrees: 0.02,
            ..a
        };
        assert!(!has_meaningful_motion(
            below,
            a,
            MotionThresholds::default()
        ));
        assert!(has_meaningful_motion(
            above_dist,
            a,
            MotionThresholds::default()
        ));
        assert!(has_meaningful_motion(
            above_angle,
            a,
            MotionThresholds::default()
        ));
        assert!(!distance_cut(above_dist, a, 0.25));
        assert!(!angle_cut(above_angle, a, 5.0));
    }

    #[test]
    fn selects_sky_matrix_for_far_depth() {
        let g = Mat4::from_translation(Vec3::X);
        let s = Mat4::IDENTITY;
        let m = ReprojectionMatrices {
            geometry: g,
            sky: s,
        };
        assert_eq!(select_reprojection(m, 0.0), s);
        assert_eq!(select_reprojection(m, FAR_DEPTH), s);
        assert_eq!(select_reprojection(m, 0.5), g);
        assert_eq!(select_reprojection(m, 1.0), g);
    }
}
