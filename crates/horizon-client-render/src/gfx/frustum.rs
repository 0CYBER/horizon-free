//! What the renderer culls with: frusta, their unions (the water mirror's, the world views') and
//! the shader pack shadow camera's box.

use super::{Frustum, SectionKey};
use crate::fast_hash::FxHashMap;
use crate::render_views::MAX_WORLD_VIEWS;
use glam::{DVec3, Mat4, Vec3, Vec4};

/// The union of two frusta (the camera's and its water mirror's), for `Resident::visible`.
#[derive(Clone, Copy, Debug)]
pub struct Frusta(pub Frustum, pub Option<Frustum>);
/// What `Resident::visible` culls with: a frustum or a union of them.
pub trait Cull {
    fn intersects(&self, min: Vec3, max: Vec3) -> bool;
    fn contains(&self, min: Vec3, max: Vec3) -> bool;
    /// Whether a section is tested by the box around its geometry, not its whole 16 blocks.
    fn tight(&self) -> bool {
        true
    }
}
impl Cull for Frustum {
    fn intersects(&self, min: Vec3, max: Vec3) -> bool {
        Frustum::intersects(self, min, max)
    }
    fn contains(&self, min: Vec3, max: Vec3) -> bool {
        Frustum::contains(self, min, max)
    }
}
impl Cull for Frusta {
    fn intersects(&self, min: Vec3, max: Vec3) -> bool {
        self.0.intersects(min, max) || self.1.is_some_and(|f| f.intersects(min, max))
    }
    /// Entirely inside one of them is entirely inside the union.
    fn contains(&self, min: Vec3, max: Vec3) -> bool {
        self.0.contains(min, max) || self.1.is_some_and(|f| f.contains(min, max))
    }
}
/// The camera's frustum joined to this frame's world views' (`Renderer::camera_frustum`): what
/// the game builds entities and block entities for is what one of them sees. Without views it
/// is the camera's alone.
#[derive(Clone, Copy, Debug)]
pub struct JoinedFrusta {
    camera: Frustum,
    views: [Frustum; MAX_WORLD_VIEWS],
    len: usize,
}
impl JoinedFrusta {
    pub(super) fn new(camera: Frustum, views: impl IntoIterator<Item = Frustum>) -> Self {
        let mut joined = Self {
            camera,
            views: [camera; MAX_WORLD_VIEWS],
            len: 0,
        };
        for (slot, view) in joined.views.iter_mut().zip(views) {
            *slot = view;
            joined.len += 1;
        }
        joined
    }
    /// The same views with the camera's frustum `camera` (TAA's wider one).
    pub(super) fn with_camera(self, camera: Frustum) -> Self {
        Self { camera, ..self }
    }
    fn views(&self) -> &[Frustum] {
        &self.views[..self.len]
    }
    /// `Frustum::intersects` for the camera's frustum or a view's.
    pub fn intersects(&self, min: Vec3, max: Vec3) -> bool {
        self.camera.intersects(min, max) || self.views().iter().any(|f| f.intersects(min, max))
    }
    /// `Frustum::intersects_sphere` for the camera's frustum or a view's.
    pub fn intersects_sphere(&self, center: Vec3, radius: f32) -> bool {
        self.camera.intersects_sphere(center, radius)
            || self
                .views()
                .iter()
                .any(|f| f.intersects_sphere(center, radius))
    }
    /// The camera's depth planes alone (`Frustum::intersects_depth`), which TAA's jitter cannot
    /// move, or a view's whole frustum, which it does not move.
    pub fn intersects_depth(&self, min: Vec3, max: Vec3) -> bool {
        self.camera.intersects_depth(min, max)
            || self.views().iter().any(|f| f.intersects(min, max))
    }
    /// Entirely inside one of them is entirely inside the union.
    pub fn contains(&self, min: Vec3, max: Vec3) -> bool {
        self.camera.contains(min, max) || self.views().iter().any(|f| f.contains(min, max))
    }
    /// `Frustum::contains_point` for the camera's frustum or a view's.
    pub fn contains_point(&self, point: Vec3) -> bool {
        self.camera.contains_point(point) || self.views().iter().any(|f| f.contains_point(point))
    }
    /// Each of them seen from a camera moved `blocks` back (`Frustum::pulled_back`).
    pub fn pulled_back(&self, blocks: f32) -> Self {
        Self {
            camera: self.camera.pulled_back(blocks),
            views: self.views.map(|f| f.pulled_back(blocks)),
            len: self.len,
        }
    }
}
/// The camera's frustum alone, with no world views.
impl From<Frustum> for JoinedFrusta {
    fn from(camera: Frustum) -> Self {
        Self::new(camera, [])
    }
}
impl Cull for JoinedFrusta {
    fn intersects(&self, min: Vec3, max: Vec3) -> bool {
        JoinedFrusta::intersects(self, min, max)
    }
    fn contains(&self, min: Vec3, max: Vec3) -> bool {
        JoinedFrusta::contains(self, min, max)
    }
}
/// A shader pack's shadow camera's box, and with `casters` only what can shade the camera's view
/// (`shadow_casters`): a box must meet all of them.
#[derive(Clone, Copy, Debug)]
pub struct ShadowCull {
    pub light: Frustum,
    pub casters: Option<[Frustum; 2]>,
    /// `shadowDistance × shadowDistanceRenderMul`: only what lies within this many blocks of the
    /// camera along each axis, a cube around it. Our probe's Iris 1.11.7 maps keep the sections
    /// whose box, grown by `SHADOW_CASTER_MARGIN` as for the casters, meets that cube (#8464):
    /// out to its corners, and a section 1 block past its side but not one 1.5 blocks past.
    pub reach: Option<f32>,
}
impl ShadowCull {
    /// Whether the camera-relative box `min..max`, grown by the margin, meets (`whole`: lies
    /// within) `reach`'s cube.
    fn within_reach(&self, min: Vec3, max: Vec3, whole: bool) -> bool {
        let (min, max) = (min - SHADOW_CASTER_MARGIN, max + SHADOW_CASTER_MARGIN);
        self.reach.is_none_or(|d| {
            if whole {
                min.cmpge(Vec3::splat(-d)).all() && max.cmple(Vec3::splat(d)).all()
            } else {
                max.cmpge(Vec3::splat(-d)).all() && min.cmple(Vec3::splat(d)).all()
            }
        })
    }
}
/// How far past a section's 16 blocks the shadow pass looks for what can shade the view: our
/// probes' Iris 1.11.7 maps keep the sections whose box, grown by this much on every side, meets
/// `shadow_casters`' shape. Over 76 scenes at fields of view 30 to 110, Iris keeps every section
/// that needs up to 1.1224 blocks and drops every one that needs 1.1252 or more (#8464): one
/// growth, whatever the field of view, of 9/8 (a fit, not Iris's code). Without it the edges of
/// the wedge lose a column of terrain.
const SHADOW_CASTER_MARGIN: f32 = 1.125;
impl Cull for ShadowCull {
    fn intersects(&self, min: Vec3, max: Vec3) -> bool {
        let (grown_min, grown_max) = (min - SHADOW_CASTER_MARGIN, max + SHADOW_CASTER_MARGIN);
        self.within_reach(min, max, false)
            && self.light.intersects(min, max)
            && self
                .casters
                .is_none_or(|c| c.iter().all(|f| f.intersects(grown_min, grown_max)))
    }
    fn contains(&self, min: Vec3, max: Vec3) -> bool {
        let (grown_min, grown_max) = (min - SHADOW_CASTER_MARGIN, max + SHADOW_CASTER_MARGIN);
        self.within_reach(min, max, true)
            && self.light.contains(min, max)
            && self
                .casters
                .is_none_or(|c| c.iter().all(|f| f.contains(grown_min, grown_max)))
    }
    /// Our probe's Iris 1.11.7 maps fit testing the casters' whole sections (a guess that matched).
    fn tight(&self) -> bool {
        self.casters.is_none()
    }
}
/// The block entities of `sections` whose box, a block wider on every side, comes within the
/// 64 blocks block entities are drawn to (`Renderer::visible_block_entities_in_frustum`).
pub(super) fn block_entities_within_reach<'a>(
    sections: impl Iterator<Item = SectionKey> + 'a,
    block_entities: &'a FxHashMap<SectionKey, Vec<([i32; 3], u32)>>,
    camera: DVec3,
    min_y: i32,
) -> impl Iterator<Item = &'a ([i32; 3], u32)> + 'a {
    sections
        .filter_map(move |key| {
            let min = (DVec3::new(
                f64::from(key.0 * 16 - 1),
                f64::from(min_y + key.1 * 16 - 1),
                f64::from(key.2 * 16 - 1),
            ) - camera)
                .as_vec3();
            let max = min + Vec3::splat(18.0);
            let near = Vec3::ZERO.max(min).min(max).length_squared() <= 64.0 * 64.0;
            near.then(|| block_entities.get(&key)).flatten()
        })
        .flatten()
}
/// The camera's view `view_proj` (wgpu depth, reversed or not) stretched without end toward
/// `light` (a direction): the boxes whose shadow, cast away from the light, can fall into the
/// view. Its planes are the view's faces turned toward the light, and through each edge between
/// such a face and one turned away, the plane along the light; unused ones take in everything.
pub(super) fn shadow_casters(view_proj: Mat4, light: Vec3) -> [Frustum; 2] {
    let faces = frustum(view_proj);
    let toward = faces.map(|p| p.truncate().dot(light) >= 0.0);
    let inverse = view_proj.inverse();
    // Corner (x, y, z) of clip space's box; `frustum`'s faces at x -1 and 1 are 0 and 1, at
    // y -1 and 1 are 2 and 3, at depth 0 and 1 are 4 and 5.
    let corner = |c: [usize; 3]| {
        inverse.project_point3(Vec3::new(
            c[0] as f32 * 2.0 - 1.0,
            c[1] as f32 * 2.0 - 1.0,
            c[2] as f32,
        ))
    };
    let corners: Vec<[usize; 3]> = (0..8).map(|i| [i & 1, (i >> 1) & 1, i >> 2]).collect();
    let centre = corners.iter().map(|&c| corner(c)).sum::<Vec3>() / 8.0;
    let mut planes = [Vec4::W; 12];
    let mut n = 0;
    for (k, &face) in faces.iter().enumerate() {
        if toward[k] {
            planes[n] = face;
            n += 1;
        }
    }
    for a in &corners {
        for axis in 0..3 {
            if a[axis] == 1 {
                continue;
            }
            let mut b = *a;
            b[axis] = 1;
            // The edge from a to b lies on the faces of the two other axes.
            let [f, g] = [(axis + 1) % 3, (axis + 2) % 3].map(|o| o * 2 + a[o]);
            if toward[f] == toward[g] {
                continue;
            }
            let (p, q) = (corner(*a), corner(b));
            let normal = (q - p).cross(light);
            if normal.length_squared() < 1e-12 {
                continue;
            }
            let normal = normal.normalize();
            let side = if normal.dot(centre - p) < 0.0 {
                -1.0
            } else {
                1.0
            };
            // At most six such edges with at most four faces toward the light: 12 planes.
            if let Some(plane) = planes.get_mut(n) {
                *plane = (normal * side).extend(-normal.dot(p) * side);
                n += 1;
            }
        }
    }
    let [a, b] = [0, 6].map(|i| Frustum {
        planes: planes[i..i + 6].try_into().expect("six planes"),
    });
    [a, b]
}
pub(super) fn taa_frustum(view_proj: Mat4, rendered_size: [u32; 2]) -> Frustum {
    // TAA shifts clip x/y by at most half a rendered pixel times clip w.
    // Widen x/y before extracting the planes: this includes every such shifted
    // view, including a posed camera, without changing the near/far planes.
    let [x, y] = rendered_size.map(|size| 1.0 / (1.0 + 2.0 / size.max(1) as f32));
    Frustum {
        planes: frustum(Mat4::from_scale(Vec3::new(x, y, 1.0)) * view_proj),
    }
}

pub(super) fn frustum(m: Mat4) -> [Vec4; 6] {
    let r = |i| m.row(i);
    let planes = [
        r(3) + r(0),
        r(3) - r(0),
        r(3) + r(1),
        r(3) - r(1),
        r(2),
        r(3) - r(2),
    ];
    planes.map(|p| p / p.truncate().length())
}
