//! BlockModelFeatureRenderer breaking quads and SheetedDecalTextureGenerator UVs.
use crate::{camera::joml, mesher::Vertex, models::catalog::BakedModel};
use horizon_assets::model::Direction;
use std::f32::consts::{FRAC_PI_2, PI};

/// Project in model space: the inverse decal pose cancels the block translation/offset.
pub fn decal_uv([x, y, z]: [f32; 3], direction: Direction) -> [f32; 2] {
    let sy = joml::sin(PI);
    let cy = joml::cos_from_sin(sy, PI);
    let sx = joml::sin(-FRAC_PI_2);
    let cx = joml::cos_from_sin(sx, -FRAC_PI_2);
    let p = [cy * x + sy * z, y, -sy * x + cy * z];
    let p = [p[0], cx * p[1] - sx * p[2], sx * p[1] + cx * p[2]];
    let (angle_x, angle_z) = match direction {
        Direction::Down => (PI, 0.0),
        Direction::Up => (0.0, 0.0),
        Direction::North => (FRAC_PI_2, PI),
        Direction::South => (FRAC_PI_2, 0.0),
        Direction::West => (FRAC_PI_2, FRAC_PI_2),
        Direction::East => (FRAC_PI_2, -FRAC_PI_2),
    };
    let p = joml::Quat::rotation_yxz(0.0, angle_x, angle_z).transform(p);
    [-p[0], -p[1]]
}

/// The light bytes of a crack vertex: the pass is unlit, so they carry the origin of the
/// vertex's destroy stage sprite (each crack in `LevelRenderer.destroyingBlocks` has its own
/// stage); its size goes in `glint_uv`, as the two sets may differ in size (`CrackSprite`).
pub fn stage_light([u0, v0]: [f32; 2]) -> u32 {
    let q = |x: f32| (x.clamp(0.0, 1.0) * 65535.0).round() as u32;
    q(u0) | (q(v0) << 16)
}

/// A crack's stage sprite, `[u0, v0, width, height]` in the atlas, for each kind of surface:
/// `SubmitNodeCollection.submitBreakingBlockModel` takes `DESTROY_TYPES_OIT` for a translucent model
/// under Improved Transparency and `DESTROY_TYPES` for any other, so a solid block's crack is
/// always the classic set.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CrackSprite {
    pub solid: [f32; 4],
    pub translucent: [f32; 4],
}

pub fn geometry(
    model: &BakedModel,
    origin: glam::Vec3,
    translucent: bool,
    stage: CrackSprite,
    out: &mut Vec<Vertex>,
) {
    // The sprite's origin in the light bytes (`stage_light`), its size in `glint_uv`, which cracks
    // don't otherwise use.
    let [u0, v0, w, h] = if translucent {
        stage.translucent
    } else {
        stage.solid
    };
    // Vanilla visits all six cull groups and then the unculled faces, even beside solid neighbours.
    for cull in Direction::ALL.map(Some).into_iter().chain([None]) {
        for face in model.faces.iter().filter(|face| face.quad.cull == cull) {
            for pos in face.quad.positions {
                out.push(Vertex {
                    pos: (origin + glam::Vec3::from(pos)).to_array(),
                    uv: decal_uv(pos, face.quad.facing),
                    layer: if translucent { 12 } else { 11 },
                    color: 0xffff_ffff,
                    light: stage_light([u0, v0]),
                    normal: [0; 4],
                    glint_uv: [w, h],
                });
            }
        }
    }
}

/// Project a block entity's transformed quads onto the same destroy-stage pass, as a solid crack
/// (`submitCrumblingOverlay`: their render types don't blend).
pub fn entity_geometry(vertices: &mut [Vertex], origin: glam::Vec3, stage: CrackSprite) {
    let [u0, v0, w, h] = stage.solid;
    for quad in vertices.as_chunks_mut::<4>().0 {
        let positions = [0, 1, 2].map(|i| glam::Vec3::from(quad[i].pos));
        let normal = (positions[1] - positions[0]).cross(positions[2] - positions[0]);
        let mut facing = Direction::North;
        let mut best = f32::from_bits(1);
        for (direction, dot) in [
            (Direction::Down, -normal.y),
            (Direction::Up, normal.y),
            (Direction::North, -normal.z),
            (Direction::South, normal.z),
            (Direction::West, -normal.x),
            (Direction::East, normal.x),
        ] {
            if dot > best {
                facing = direction;
                best = dot;
            }
        }
        for vertex in quad {
            vertex.uv = decal_uv((glam::Vec3::from(vertex.pos) - origin).to_array(), facing);
            vertex.layer = 11;
            vertex.color = 0xffff_ffff;
            vertex.light = stage_light([u0, v0]);
            vertex.glint_uv = [w, h];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_origin_survives_the_light_bytes() {
        // entity.wgsl's `wrap`: the low and high 16 bits over 65535.
        for origin in [[0.0, 0.0], [0.5, 0.25], [0.123_456, 0.987_654], [1.0, 1.0]] {
            let light = stage_light(origin);
            let back = [
                (light & 0xffff) as f32 / 65535.0,
                (light >> 16) as f32 / 65535.0,
            ];
            for k in 0..2 {
                assert!(
                    (back[k] - origin[k]).abs() <= 0.5 / 65535.0,
                    "{origin:?} -> {back:?}"
                );
            }
        }
    }
    use crate::models::catalog::BakedFace;
    use horizon_assets::{bake::ModelQuad, model::Material};
    fn slab_top() -> BakedModel {
        BakedModel {
            ambient_occlusion: true,
            particle: None,
            cube: None,
            faces: vec![BakedFace::new(
                ModelQuad {
                    positions: [
                        [0.0, 0.5, 0.0],
                        [0.0, 0.5, 1.0],
                        [1.0, 0.5, 1.0],
                        [1.0, 0.5, 0.0],
                    ],
                    uv: [[0.0; 2]; 4],
                    facing: Direction::Up,
                    cull: None,
                    material: Material {
                        sprite: "block/stone".into(),
                        force_translucent: false,
                    },
                    tint_index: -1,
                    shade_direction_override: None,
                    light_emission: 0,
                    light: [0; 2],
                    color: None,
                    ambient_occlusion: None,
                    render_type: None,
                    shade: true,
                },
                0,
                true,
                false,
                None,
            )],
        }
    }
    #[test]
    fn slab_overlay_follows_baked_top_not_unit_cube() {
        let model = slab_top();
        let mut vertices = Vec::new();
        geometry(
            &model,
            glam::Vec3::new(2.0, 3.0, 4.0),
            false,
            CrackSprite::default(),
            &mut vertices,
        );
        assert_eq!(vertices.len(), 4);
        for (v, p) in vertices.iter().zip(model.faces[0].quad.positions) {
            assert_eq!(v.pos, [p[0] + 2.0, p[1] + 3.0, p[2] + 4.0]);
            assert_eq!(v.color, 0xffff_ffff);
        }
    }
    #[test]
    fn entity_overlay_uses_block_local_projected_uvs() {
        let origin = glam::Vec3::new(12.0, -3.0, 8.0);
        let mut expected = Vec::new();
        geometry(
            &slab_top(),
            origin,
            false,
            CrackSprite::default(),
            &mut expected,
        );
        let mut actual = expected.clone();
        for vertex in &mut actual {
            vertex.uv = [0.0; 2];
            vertex.layer = 0;
            vertex.color = 0;
        }
        entity_geometry(&mut actual, origin, CrackSprite::default());
        assert_eq!(
            bytemuck::cast_slice::<_, u8>(&actual),
            bytemuck::cast_slice::<_, u8>(&expected)
        );
    }

    #[test]
    fn small_faces_use_projected_not_stretched_uvs() {
        let a = decal_uv([0.0, 0.0, 0.0], Direction::North);
        let b = decal_uv([0.0, 0.5, 0.0], Direction::North);
        assert!(((b[1] - a[1]).abs() - 0.5).abs() < 1e-6);
    }
    #[test]
    fn translucent_overlay_has_its_own_pass_and_sprite() {
        let stage = CrackSprite {
            solid: [0.0; 4],
            translucent: [0.5, 0.25, 0.125, 0.0625],
        };
        let mut vertices = Vec::new();
        geometry(&slab_top(), glam::Vec3::ZERO, true, stage, &mut vertices);
        assert!(vertices.iter().all(|v| v.layer == 12
            && v.light == stage_light([0.5, 0.25])
            && v.glint_uv == [0.125, 0.0625]));
    }
    #[test]
    fn empty_model_has_no_cube_overlay() {
        let mut model = slab_top();
        model.faces.clear();
        let mut vertices = Vec::new();
        geometry(
            &model,
            glam::Vec3::ZERO,
            false,
            CrackSprite::default(),
            &mut vertices,
        );
        assert!(vertices.is_empty());
    }
}
