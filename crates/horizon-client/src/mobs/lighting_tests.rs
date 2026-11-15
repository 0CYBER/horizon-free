// Test module extracted without changing its logical path.
use super::*;
use glam::{Mat4, Vec3};
use serde_json::json;

fn plane(normal: [f32; 3]) -> Part {
    Part::parse(
        "root",
        &json!({
            "polygons": [[[0,0,0,0,0], [0,0,0,1,0], [0,0,0,1,1], [0,0,0,0,1]]],
            "normals": [normal]
        }),
    )
}

#[test]
fn exported_normals_survive_collapsed_faces_and_mirroring() {
    let part = plane([1.0, 0.0, 0.0]);
    assert_eq!(
        part.polygons[0].vertex_normal(ModelNormals::new(Mat4::IDENTITY)),
        [127, 0, 0, -127]
    );
    assert_eq!(
        part.polygons[0].vertex_normal(ModelNormals::new(Mat4::from_scale(Vec3::new(
            -1.0, -1.0, 1.0
        )))),
        [-127, 0, 0, -127]
    );
}

#[test]
fn normal_matrix_includes_parent_scale_and_child_rotation() {
    let mut child = plane([1.0, 0.0, 0.0]);
    child.pose[5] = std::f32::consts::FRAC_PI_4;
    let parent = Part {
        name: "parent".into(),
        pose: [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 2.0, 1.0, 0.5],
        polygons: Vec::new(),
        children: vec![child],
    };
    let mut animated = Vec::new();
    render::draw(
        &parent,
        Mat4::IDENTITY,
        &anim::Poses::new(&parent),
        [0.0, 0.0, 1.0, 1.0],
        0x80402010,
        240,
        1,
        &mut animated,
    );
    let mut direct = Vec::new();
    render_model(
        &parent,
        Mat4::IDENTITY,
        &|p| p.pose,
        [0.0, 0.0, 1.0, 1.0],
        240,
        1,
        &mut direct,
    );
    assert_eq!(animated.len(), 4);
    for (a, b) in animated.iter().zip(&direct) {
        assert_eq!(a.normal, [56, 113, 0, -127]);
        assert_eq!(a.normal, b.normal);
        assert_eq!(a.pos, b.pos);
        assert_eq!(a.uv, b.uv);
        assert_eq!((a.color, a.light, a.layer), (0x80402010, 240, 1));
    }
}

#[test]
fn trusted_rotation_retains_native_quaternion_rounding() {
    let mut part = plane([1.0, 0.0, 0.0]);
    part.pose[3] = std::f32::consts::FRAC_PI_2;
    let mut vertices = Vec::new();
    render::draw(
        &part,
        Mat4::IDENTITY,
        &anim::Poses::new(&part),
        [0.0, 0.0, 1.0, 1.0],
        0xffffffff,
        240,
        0,
        &mut vertices,
    );
    assert_eq!(vertices[0].normal, [126, 0, 0, -127]);
}

#[test]
fn legacy_models_infer_non_degenerate_face_normals() {
    let part = Part::parse(
        "root",
        &json!({"polygons": [
            [[0,0,0,0,0], [1,0,0,1,0], [1,1,0,1,1], [0,1,0,0,1]]
        ]}),
    );
    assert_eq!(part.polygons[0].normal, [0.0, 0.0, 1.0]);
}

#[test]
fn packed_normal_uses_native_snorm_bytes_without_changing_vertex_offsets() {
    assert_eq!(std::mem::offset_of!(Vertex, normal), 32);
    assert_eq!(std::mem::size_of::<Vertex>(), 44);
    let part = plane([0.0, 1.0, 0.0]);
    assert_eq!(
        part.polygons[0].vertex_normal(ModelNormals::new(Mat4::from_scale(Vec3::ZERO))),
        [0, 0, 0, -127]
    );
}
