//! `GuiGraphics.renderItem` drawn into the HUD batch: the item's layers are
//! submitted under the GUI item pose (`translate(x + 8, y + 8)`,
//! `scale(16, -16, 16)`), lit with `Lighting.Entry.ITEMS_3D` or `ITEMS_FLAT`
//! (`minecraft_mix_light` on the transformed normals), back faces culled,
//! drawn far to near, and clipped to the item's 16x16 slot unless the model
//! is oversized in the GUI (`GuiItemAtlas` draws each item into a scissored
//! 16x16 slot; oversized items get a picture-in-picture of their bounds).
use super::{Foil, ItemQuad, ItemRenderState, ItemRenderer, SpecialHost};
use crate::hud::{HudBatch, Rect};
use glam::{EulerRot, Mat4, Vec2, Vec3};
use std::{collections::HashMap, f32::consts::PI};

/// `Lighting.DIFFUSE_LIGHT_0` and `DIFFUSE_LIGHT_1`.
fn diffuse_lights() -> (Vec3, Vec3) {
    (
        Vec3::new(0.2, 1.0, -0.7).normalize(),
        Vec3::new(-0.2, 1.0, 0.7).normalize(),
    )
}
/// `Lighting.Entry.ITEMS_FLAT`: `rotationY(-PI/8).rotateX(3PI/4)`.
pub fn items_flat_lights() -> (Vec3, Vec3) {
    let m = Mat4::from_rotation_y(-PI / 8.0) * Mat4::from_rotation_x(PI * 3.0 / 4.0);
    let (a, b) = diffuse_lights();
    (m.transform_vector3(a), m.transform_vector3(b))
}
/// JOML `rotateYXZ(y, x, z)`.
fn rotate_yxz(y: f32, x: f32, z: f32) -> Mat4 {
    Mat4::from_euler(EulerRot::YXZ, y, x, z)
}
/// `Lighting.Entry.ITEMS_3D`.
pub fn items_3d_lights() -> (Vec3, Vec3) {
    let m = Mat4::from_scale(Vec3::new(1.0, -1.0, 1.0))
        * rotate_yxz(1.082_104_1, 3.237_585_8, 0.0)
        * rotate_yxz(-PI / 8.0, PI * 3.0 / 4.0, 0.0);
    let (a, b) = diffuse_lights();
    (m.transform_vector3(a), m.transform_vector3(b))
}

/// A GUI-space vertex: position, atlas UV.
#[derive(Clone, Copy, Debug)]
struct Point {
    p: Vec2,
    uv: [f32; 2],
}
fn lerp(a: Point, b: Point, t: f32) -> Point {
    Point {
        p: a.p + (b.p - a.p) * t,
        uv: [
            a.uv[0] + (b.uv[0] - a.uv[0]) * t,
            a.uv[1] + (b.uv[1] - a.uv[1]) * t,
        ],
    }
}
/// Sutherland-Hodgman against one half plane `dot(p, n) <= limit`.
fn clip_edge(poly: &[Point], axis: usize, limit: f32, keep_below: bool) -> Vec<Point> {
    let inside = |p: &Point| {
        let v = p.p[axis];
        if keep_below { v <= limit } else { v >= limit }
    };
    let mut out = Vec::with_capacity(poly.len() + 2);
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        let (ia, ib) = (inside(&a), inside(&b));
        if ia {
            out.push(a);
        }
        if ia != ib {
            let t = (limit - a.p[axis]) / (b.p[axis] - a.p[axis]);
            out.push(lerp(a, b, t));
        }
    }
    out
}
/// A triangle clipped to `[x0, x1] x [y0, y1]`, as a fan.
fn clip_triangle(tri: [Point; 3], rect: [f32; 4]) -> Vec<Point> {
    let mut poly = tri.to_vec();
    for (axis, limit, below) in [
        (0, rect[0], false),
        (0, rect[2], true),
        (1, rect[1], false),
        (1, rect[3], true),
    ] {
        if poly.is_empty() {
            break;
        }
        poly = clip_edge(&poly, axis, limit, below);
    }
    poly
}

/// RGBA vertex colour of a lit quad: `minecraft_mix_light` on the tint.
fn lit_color(q: &ItemQuad, lights: (Vec3, Vec3)) -> u32 {
    let light = crate::block_entity_render::diffuse(q.normal, lights);
    let [a, r, g, b] = q.color.to_be_bytes();
    let c = |v: u8| (f32::from(v) * light) as u8;
    crate::hud::rgba(c(r), c(g), c(b), a)
}
/// The glint's moving sheen at a GUI position (a stand-in for the
/// `GLINT_TEXTURING`-scrolled `enchanted_glint_item` texture added on top).
fn glint_color(p: Vec2, time: f32) -> u32 {
    let d = ((p.x + p.y) / 32.0 - time * 0.35).rem_euclid(1.0);
    let band = (1.0 - (d - 0.5).abs() * 6.0).max(0.0);
    crate::hud::rgba(
        (170.0 + 70.0 * band) as u8,
        (90.0 + 110.0 * band) as u8,
        255,
        (45.0 + 110.0 * band) as u8,
    )
}

fn emit(batch: &mut HudBatch, fan: &[Point], color: impl Fn(Vec2) -> u32, glint: bool) {
    let shift = if glint { 2.0 } else { 0.0 };
    for i in 1..fan.len().saturating_sub(1) {
        let (a, b, c) = (fan[0], fan[i], fan[i + 1]);
        let corners = [a, b, c, c];
        // `HudBatch.quad` draws (0, 1, 2) and (0, 2, 3): the second is empty.
        batch.quad(
            corners.map(|v| v.p.to_array()),
            corners.map(|v| [v.uv[0] + shift, v.uv[1]]),
            color(a.p),
        );
        if glint {
            // Per-vertex colours are flat per quad in the HUD batch: use the
            // fan centre for the sheen.
            let n = batch.vertices.len();
            for v in &mut batch.vertices[n - 6..] {
                v.color = color(Vec2::from(v.pos));
            }
        }
    }
}

/// Submit, light, cull, sort and clip one item's quads into `batch` at GUI
/// position `(x, y)`.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    state: &ItemRenderState,
    renderer: &ItemRenderer,
    sprites: &HashMap<String, Rect>,
    host: &dyn SpecialHost,
    batch: &mut HudBatch,
    x: f32,
    y: f32,
    time: f32,
) {
    if state.is_empty() {
        return;
    }
    let pose = Mat4::from_translation(Vec3::new(x + 8.0, y + 8.0, 0.0))
        * Mat4::from_scale(Vec3::new(16.0, -16.0, 16.0));
    let mut quads: Vec<ItemQuad> = Vec::new();
    state.submit(renderer, sprites, host, pose, &mut quads);
    let lights = if state.uses_block_light() {
        items_3d_lights()
    } else {
        items_flat_lights()
    };
    // Back-face culling: front faces wind counter-clockwise on screen with y up.
    let mut visible: Vec<(f32, usize)> = quads
        .iter()
        .enumerate()
        .filter(|(_, q)| {
            let p = q.pos.map(|v| Vec2::new(v.x, -v.y));
            let area: f32 = (0..4)
                .map(|i| {
                    let (a, b) = (p[i], p[(i + 1) % 4]);
                    a.x * b.y - b.x * a.y
                })
                .sum();
            area > 0.0
        })
        .map(|(i, q)| (q.pos.iter().map(|v| v.z).sum::<f32>() / 4.0, i))
        .collect();
    // Far to near; the depth buffer of the item atlas, approximated per quad.
    visible.sort_by(|a, b| a.0.total_cmp(&b.0));
    let rect = if state.oversized_in_gui {
        [
            f32::NEG_INFINITY,
            f32::NEG_INFINITY,
            f32::INFINITY,
            f32::INFINITY,
        ]
    } else {
        [x, y, x + 16.0, y + 16.0]
    };
    for (_, i) in visible {
        let q = &quads[i];
        let points: [Point; 4] = std::array::from_fn(|k| Point {
            p: Vec2::new(q.pos[k].x, q.pos[k].y),
            uv: q.uv[k],
        });
        let color = lit_color(q, lights);
        for tri in [
            [points[0], points[1], points[2]],
            [points[0], points[2], points[3]],
        ] {
            let fan = clip_triangle(tri, rect);
            if fan.len() < 3 {
                continue;
            }
            emit(batch, &fan, |_| color, false);
            if q.foil != Foil::None {
                emit(batch, &fan, |p| glint_color(p, time), true);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_lights_match_joml() {
        // new Matrix4f().rotationY(-PI/8).rotateX(3PI/4).transformDirection(L0).
        let (l0, _) = items_flat_lights();
        let l = Vec3::new(0.2, 1.0, -0.7).normalize();
        let (s, c) = ((3.0 * PI / 4.0).sin(), (3.0 * PI / 4.0).cos());
        let rx = Vec3::new(l.x, c * l.y - s * l.z, s * l.y + c * l.z);
        let (s, c) = ((-PI / 8.0).sin(), (-PI / 8.0).cos());
        let ry = Vec3::new(c * rx.x + s * rx.z, rx.y, -s * rx.x + c * rx.z);
        assert!((l0 - ry).length() < 1e-6);
    }
    #[test]
    fn triangles_clip_to_the_slot() {
        let p = |x: f32, y: f32| Point {
            p: Vec2::new(x, y),
            uv: [x, y],
        };
        let fan = clip_triangle(
            [p(-4.0, 0.0), p(8.0, 0.0), p(8.0, 8.0)],
            [0.0, 0.0, 16.0, 16.0],
        );
        assert!(fan.iter().all(|v| v.p.x >= 0.0));
        // UVs follow the cut affinely.
        assert!(fan.iter().all(|v| (v.uv[0] - v.p.x).abs() < 1e-6));
        assert!(
            clip_triangle(
                [p(20.0, 0.0), p(30.0, 0.0), p(30.0, 5.0)],
                [0.0, 0.0, 16.0, 16.0]
            )
            .is_empty()
        );
    }
}
