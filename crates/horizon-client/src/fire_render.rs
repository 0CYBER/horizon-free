//! Animated burning-entity flames; the first-person overlay uses screen_effects.
use crate::{hud::Rect, mesher::Vertex};
use std::collections::HashMap;

#[allow(clippy::too_many_arguments)]
pub fn flames(
    position: [f64; 3],
    width: f32,
    height: f32,
    camera: glam::DVec3,
    yaw: f32,
    sprites: &HashMap<String, Rect>,
    scene_light: u8,
    out: &mut Vec<Vertex>,
) {
    let (Some(a), Some(b)) = (sprites.get("block/fire_0"), sprites.get("block/fire_1")) else {
        return;
    };
    let scale = width * 1.4;
    if scale <= 0.0 {
        return;
    }
    let origin = glam::DVec3::from(position) - camera;
    let rotation = glam::Mat3::from_rotation_y(std::f32::consts::PI - yaw.to_radians());
    let mut remaining = height / scale;
    let mut half = 0.5f32;
    let mut y = 0.0f32;
    let mut z = 0.3 - (remaining as i32) as f32 * 0.02;
    let mut index = 0;
    while remaining > 0.0 {
        let r = if index % 2 == 0 { a } else { b };
        let (u0, u1) = if (index / 2) % 2 == 0 {
            (r.u0, r.u1)
        } else {
            (r.u1, r.u0)
        };
        for (p, uv) in [
            ([-half, y, z], [u0, r.v1]),
            ([half, y, z], [u1, r.v1]),
            ([half, y + 1.4, z], [u1, r.v0]),
            ([-half, y + 1.4, z], [u0, r.v0]),
        ] {
            let p = origin.as_vec3() + rotation * (glam::Vec3::from(p) * scale);
            out.push(Vertex {
                pos: p.to_array(),
                uv,
                layer: 0,
                color: 0xffff_ffff,
                light: 0xf000 | (u32::from(scene_light >> 4) * 16),
                normal: [0; 4],

                glint_uv: [0.0, 0.0],
            });
        }
        remaining -= 0.45;
        y += 0.45;
        half *= 0.9;
        z -= 0.03;
        index += 1;
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn flames_cover_entity_height_and_alternate_animated_sprites() {
        let rect = |u0| super::Rect {
            u0,
            v0: 0.0,
            u1: u0 + 0.1,
            v1: 0.1,
            w: 16,
            h: 16,
        };
        let sprites = super::HashMap::from([
            ("block/fire_0".into(), rect(0.0)),
            ("block/fire_1".into(), rect(0.5)),
        ]);
        let mut out = Vec::new();
        super::flames(
            [0.0; 3],
            0.6,
            1.8,
            glam::DVec3::ZERO,
            0.0,
            &sprites,
            0,
            &mut out,
        );
        assert_eq!(out.len() % 4, 0);
        assert!(
            out[0].pos[2] < 0.0,
            "billboard depth bias must face the camera"
        );
        assert!(
            out.iter().all(|v| v.light == 0xf000),
            "fire forces block light to 15 but preserves scene sky light"
        );
        assert!(out.len() >= 20);
        assert!(out.iter().any(|v| v.pos[1] >= 1.8));
        assert!(
            out.iter().all(|v| v.layer == 0),
            "flames must not use the breaking-cracks UV-remap material"
        );
        assert!(out[..4].iter().all(|v| v.uv[0] <= 0.1));
        assert!(out[4..8].iter().all(|v| v.uv[0] >= 0.5));
    }
}
