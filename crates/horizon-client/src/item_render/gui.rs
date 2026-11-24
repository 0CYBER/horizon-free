//! `GuiGraphics.renderItem` drawn into the HUD batch: the item's layers are
//! submitted under the GUI item pose (`translate(x + 8, y + 8)`,
//! `scale(16, -16, 16)`), lit with `Lighting.Entry.ITEMS_3D` or `ITEMS_FLAT`
//! (`minecraft_mix_light` on the transformed normals), back faces culled,
//! drawn far to near, and clipped to the item's 16x16 slot unless the model
//! is oversized in the GUI (`GuiItemAtlas` draws each item into a scissored
//! 16x16 slot; oversized items get a picture-in-picture of their bounds).
use super::{Foil, Glint, ItemQuad, ItemRenderState, ItemRenderer, SpecialHost};
use crate::cit::Rule;
use crate::cit_glint::StackGlint;
use crate::hud::cit_glint::{GlintTextures, layer_uv, vertex_color};
use crate::hud::{HudBatch, Rect};
use glam::{EulerRot, Mat4, Vec2, Vec3};
use std::{collections::HashMap, f32::consts::PI, sync::Arc};

mod depth;

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
    z: f32,
    uv: [f32; 2],
    glint_uv: [f32; 2],
    cit_uv: [f32; 2],
}
fn lerp(a: Point, b: Point, t: f32) -> Point {
    Point {
        p: a.p + (b.p - a.p) * t,
        z: a.z + (b.z - a.z) * t,
        uv: [
            a.uv[0] + (b.uv[0] - a.uv[0]) * t,
            a.uv[1] + (b.uv[1] - a.uv[1]) * t,
        ],
        cit_uv: std::array::from_fn(|k| a.cit_uv[k] + (b.cit_uv[k] - a.cit_uv[k]) * t),
        glint_uv: [
            a.glint_uv[0] + (b.glint_uv[0] - a.glint_uv[0]) * t,
            a.glint_uv[1] + (b.glint_uv[1] - a.glint_uv[1]) * t,
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
pub fn lit_color(q: &ItemQuad, lights: (Vec3, Vec3)) -> u32 {
    let light = crate::block_entity_render::diffuse(q.normal, lights);
    let [a, r, g, b] = q.color.to_be_bytes();
    let c = |v: u8| (f32::from(v) * light) as u8;
    crate::hud::rgba(c(r), c(g), c(b), a)
}
fn emit(batch: &mut HudBatch, fan: &[Point], color: impl Fn(Vec2) -> u32, glint: Option<&Glint>) {
    for i in 1..fan.len().saturating_sub(1) {
        let (a, b, c) = (fan[0], fan[i], fan[i + 1]);
        let corners = [a, b, c, c];
        if let Some(glint) = glint {
            let start = batch.vertices.len();
            batch.quad_glint(
                corners.map(|v| v.p.to_array()),
                corners.map(|v| [v.uv[0] + 2.0, v.uv[1]]),
                corners.map(|v| glint.uv(v.glint_uv)),
                u32::MAX,
            );
            batch.blend_since(start, crate::hud::Blend::Additive);
        } else {
            batch.quad(
                corners.map(|v| v.p.to_array()),
                corners.map(|v| v.uv),
                color(a.p),
            );
        }
    }
}

/// A stack's OptiFine CIT glint layers this frame.
#[derive(Clone, Copy)]
struct CitDraw<'a> {
    glint: &'a StackGlint,
    textures: &'a GlintTextures,
    rules: &'a [Rule],
    millis: f32,
}

/// The CIT glint layers over the item's clipped faces (`fans`, in draw order), bottom first:
/// each layer is one run over all of them, blended as its rule says, at its intensity times
/// the glint strength option (`HudBatch::glint_alpha`).
fn emit_cit(batch: &mut HudBatch, fans: &[Vec<Point>], cit: CitDraw) {
    let seconds = f64::from(cit.millis) / 1000.0;
    for (i, layer) in cit.glint.layers.iter().enumerate() {
        let Some(slot) = cit.textures.slot_of.get(layer.rule).copied().flatten() else {
            continue;
        };
        let Some(constants) = cit.textures.slots.get(usize::from(slot)) else {
            continue;
        };
        let intensity = cit.glint.intensity(i, seconds) * batch.glint_alpha;
        if intensity <= 0.0 {
            continue;
        }
        let glint = &cit.rules[layer.rule].glint;
        let color = vertex_color(slot, intensity);
        let uv = |v: Point| layer_uv(v.cit_uv, cit.millis, glint.speed, glint.rotation, 8.0);
        let start = batch.vertices.len();
        for fan in fans {
            for k in 1..fan.len().saturating_sub(1) {
                let corners = [fan[0], fan[k], fan[k + 1], fan[k + 1]];
                batch.quad_glint(
                    corners.map(|v| v.p.to_array()),
                    corners.map(|v| v.uv),
                    corners.map(uv),
                    color,
                );
            }
        }
        batch.blend_since(start, crate::hud::Blend::CitGlint(constants.blend));
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
    glint_speed: f32,
    glint_alpha: f32,
) {
    if state.is_empty() {
        return;
    }
    // A static item drawn where it was drawn before gives the same vertices:
    // take them instead of submitting, sorting and clipping its quads again.
    // The GUI model batch also carries material runs; the vertex-only cache is native.
    let key = (!renderer.core_gui_enabled())
        .then(|| DrawnKey::of(state, renderer, sprites, x, y))
        .flatten();
    if let Some(key) = &key
        && let Ok(drawn) = renderer.gui_drawn.lock()
        && let Some(verts) = drawn.get(key)
    {
        batch.vertices.extend_from_slice(verts);
        return;
    }
    let start = batch.vertices.len();
    draw_now(
        state,
        renderer,
        sprites,
        host,
        batch,
        x,
        y,
        time,
        glint_speed,
        glint_alpha,
        None,
    );
    if let Some(key) = key
        && let Ok(mut drawn) = renderer.gui_drawn.lock()
    {
        // GUI positions and items are few; a long scroll through every tab stays bounded.
        if drawn.len() >= DRAWN_LIMIT {
            drawn.clear();
        }
        drawn.insert(key, batch.vertices[start..].into());
    }
}

/// [`draw`] clipped to `scissor` (`[x0, y0, x1, y1]`) as well as the item's slot:
/// a list's scissor cuts the item of a half-visible row exactly. Not cached, the
/// scissor is not part of the cache key.
#[allow(clippy::too_many_arguments)]
pub fn draw_scissored(
    state: &ItemRenderState,
    renderer: &ItemRenderer,
    sprites: &HashMap<String, Rect>,
    host: &dyn SpecialHost,
    batch: &mut HudBatch,
    x: f32,
    y: f32,
    scissor: [f32; 4],
) {
    if state.is_empty() {
        return;
    }
    draw_now(
        state,
        renderer,
        sprites,
        host,
        batch,
        x,
        y,
        0.0,
        0.0,
        0.0,
        Some(scissor),
    );
}

/// Entries of `ItemRenderer::gui_drawn` before it starts over.
const DRAWN_LIMIT: usize = 8192;

/// Tint values for one cached GUI item layer: up to 2 tints inline without heap
/// allocation (covering untinted, single-tint, and 2-tint items), spilling to a
/// boxed slice only for 3+ tints.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
enum DrawnTints {
    Inline { len: u8, values: [i32; 2] },
    Heap(Box<[i32]>),
}
impl DrawnTints {
    fn of(tints: &[i32]) -> Self {
        match tints {
            [] => Self::Inline {
                len: 0,
                values: [0, 0],
            },
            [a] => Self::Inline {
                len: 1,
                values: [*a, 0],
            },
            [a, b] => Self::Inline {
                len: 2,
                values: [*a, *b],
            },
            more => Self::Heap(more.into()),
        }
    }
}

/// One static baked layer in a [`DrawnKey`].
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct DrawnLayer {
    baked: usize,
    tints: DrawnTints,
    matrix: [u32; 16],
    uses_block_light: bool,
}
impl DrawnLayer {
    fn of(layer: &super::LayerRender) -> Option<Self> {
        match &layer.geometry {
            super::LayerGeometry::Quads(baked) if layer.foil == super::FoilType::None => {
                Some(Self {
                    baked: Arc::as_ptr(baked) as usize,
                    tints: DrawnTints::of(&layer.tints),
                    matrix: layer.matrix.to_cols_array().map(f32::to_bits),
                    uses_block_light: layer.uses_block_light,
                })
            }
            _ => None,
        }
    }
}

/// Cached layer keys: 0, 1, or 2 layers inline (zero heap allocations for nearly every
/// hotbar and inventory item), spilling to a boxed slice only for 3+ layers.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
enum DrawnLayers {
    Empty,
    One(DrawnLayer),
    Two([DrawnLayer; 2]),
    Many(Box<[DrawnLayer]>),
}
impl DrawnLayers {
    fn of(layers: &[super::LayerRender]) -> Option<Self> {
        match layers {
            [] => Some(Self::Empty),
            [a] => Some(Self::One(DrawnLayer::of(a)?)),
            [a, b] => Some(Self::Two([DrawnLayer::of(a)?, DrawnLayer::of(b)?])),
            many => {
                let collected = many
                    .iter()
                    .map(DrawnLayer::of)
                    .collect::<Option<Vec<_>>>()?;
                Some(Self::Many(collected.into_boxed_slice()))
            }
        }
    }
}

/// What a GUI item's vertices depend on: its layers' baked models (kept for the
/// renderer's life, so their addresses name them), tints and transforms, the
/// atlas, and the exact position. None for items whose drawing changes with time
/// or the host: a glint, or a special model renderer.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub(crate) struct DrawnKey {
    renderer: usize,
    sprites: (usize, usize),
    at: [u32; 2],
    oversized: bool,
    layers: DrawnLayers,
}
impl DrawnKey {
    fn of(
        state: &ItemRenderState,
        renderer: &ItemRenderer,
        sprites: &HashMap<String, Rect>,
        x: f32,
        y: f32,
    ) -> Option<Self> {
        if state.animated || state.cit_glint.is_some() {
            return None;
        }
        let layers = DrawnLayers::of(&state.layers)?;
        Some(Self {
            renderer: std::ptr::from_ref(renderer) as usize,
            sprites: (std::ptr::from_ref(sprites) as usize, sprites.len()),
            at: [x.to_bits(), y.to_bits()],
            oversized: state.oversized_in_gui,
            layers,
        })
    }
}

/// [`draw`] without the cache.
#[allow(clippy::too_many_arguments)]
fn draw_now(
    state: &ItemRenderState,
    renderer: &ItemRenderer,
    sprites: &HashMap<String, Rect>,
    host: &dyn SpecialHost,
    batch: &mut HudBatch,
    x: f32,
    y: f32,
    time: f32,
    glint_speed: f32,
    glint_alpha: f32,
    scissor: Option<[f32; 4]>,
) {
    let pose = Mat4::from_translation(Vec3::new(x + 8.0, y + 8.0, 0.0))
        * Mat4::from_scale(Vec3::new(16.0, -16.0, 16.0));
    let mut quads: Vec<ItemQuad> = Vec::new();
    state.submit(renderer, sprites, host, pose, &mut quads);
    let block_light = state.uses_block_light();
    let lights = if block_light {
        items_3d_lights()
    } else {
        items_flat_lights()
    };
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
    let rect = match scissor {
        Some([x0, y0, x1, y1]) => [
            rect[0].max(x0),
            rect[1].max(y0),
            rect[2].min(x1),
            rect[3].min(y1),
        ],
        None => rect,
    };
    let glint = sprites
        .contains_key("misc/enchanted_glint_item")
        .then_some(Glint {
            speed: glint_speed,
            alpha: glint_alpha,
            millis: time * 1000.0,
            scale: 8.0,
        });
    batch.glint_alpha = glint_alpha;
    let cit = state.cit_glint.as_deref().zip(renderer.cit.as_ref());
    let cit = cit.map(|(glint, models)| CitDraw {
        glint,
        textures: &models.glint_textures,
        rules: &models.rules.rules,
        millis: time * 1000.0,
    });
    draw_quads_with(
        &quads,
        lights,
        batch,
        rect,
        glint.as_ref(),
        cit,
        renderer.core_gui_enabled().then_some(block_light),
    );
}

/// Lights, culls, sorts far to near and clips `quads` (GUI positions) to
/// `rect`.
fn draw_quads(
    quads: &[ItemQuad],
    lights: (Vec3, Vec3),
    batch: &mut HudBatch,
    rect: [f32; 4],
    glint: Option<&Glint>,
) {
    draw_quads_with(quads, lights, batch, rect, glint, None, None);
}

/// [`draw_quads`], then a stack's CIT glint layers over every face.
fn draw_quads_with(
    quads: &[ItemQuad],
    lights: (Vec3, Vec3),
    batch: &mut HudBatch,
    rect: [f32; 4],
    glint: Option<&Glint>,
    cit: Option<CitDraw>,
    pack_light: Option<bool>,
) {
    // The faces the CIT layers go over once the item is drawn.
    let mut fans = Vec::new();
    for face in depth::sort(quads) {
        let q = &quads[face.quad];
        let points: Vec<_> = face
            .corners
            .iter()
            .map(|v| Point {
                p: v.pos.truncate(),
                z: v.pos.z,
                uv: v.uv,
                glint_uv: v.glint_uv,
                cit_uv: v.cit_uv,
            })
            .collect();
        let color = lit_color(q, lights);
        for i in 1..points.len().saturating_sub(1) {
            let tri = [points[0], points[i], points[i + 1]];
            let fan = clip_triangle(tri, rect);
            if fan.len() < 3 {
                continue;
            }
            if let Some(block_light) = pack_light
                && cit.is_none()
                && q.foil == Foil::None
                && q.gui_glint_uv.is_some()
            {
                let [a, red, green, blue] = q.color.to_be_bytes();
                let tint = crate::hud::rgba(red, green, blue, a);
                for j in 1..fan.len() - 1 {
                    let corners = [fan[0], fan[j], fan[j + 1], fan[j + 1]];
                    batch.item_model_quad(
                        corners.map(|v| [v.p.x, v.p.y, v.z]),
                        corners.map(|v| v.glint_uv),
                        q.normal.to_array(),
                        tint,
                        block_light,
                    );
                }
            } else {
                emit(batch, &fan, |_| color, None);
            }
            if q.foil != Foil::None {
                emit(batch, &fan, |_| color, glint);
            }
            if cit.is_some() {
                fans.push(fan);
            }
        }
    }
    if let Some(cit) = cit {
        emit_cit(batch, &fans, cit);
    }
}

/// `PictureInPictureRenderer.prepare` for the loom's 20x40 result rectangle
/// at `(x0, y0)` (`translate(w / 2, h)`, `scale(16, 16, -16)`, one texel per
/// GUI pixel), then `GuiBannerResultRenderer.renderToTexture`'s
/// `translate(0, 0.25, 0)`.
pub fn banner_result_pose(x0: f32, y0: f32) -> Mat4 {
    Mat4::from_translation(Vec3::new(x0 + 10.0, y0 + 40.0, 0.0))
        * Mat4::from_scale(Vec3::new(16.0, 16.0, -16.0))
        * Mat4::from_translation(Vec3::new(0.0, 0.25, 0.0))
}

/// The quads of `GuiGraphicsExtractor.bannerPattern` (the loom's result
/// preview) at `(x0, y0)`: base dye and `(texture, dye)` layers.
pub fn banner_result_quads(
    renderer: &ItemRenderer,
    sprites: &HashMap<String, Rect>,
    x0: f32,
    y0: f32,
    base: &str,
    layers: &[(String, String)],
) -> Vec<ItemQuad> {
    let mut quads = Vec::new();
    super::special::submit_banner_result(
        renderer,
        banner_result_pose(x0, y0),
        base,
        layers,
        sprites,
        &mut quads,
    );
    quads
}

/// Draws the loom's result preview: the picture in picture is lit with
/// `Lighting.Entry.ITEMS_FLAT` and holds only its 20x40 rectangle.
pub fn draw_banner_result(
    renderer: &ItemRenderer,
    sprites: &HashMap<String, Rect>,
    batch: &mut HudBatch,
    x0: f32,
    y0: f32,
    base: &str,
    layers: &[(String, String)],
) {
    let quads = banner_result_quads(renderer, sprites, x0, y0, base, layers);
    draw_quads(
        &quads,
        items_flat_lights(),
        batch,
        [x0, y0, x0 + 20.0, y0 + 40.0],
        None,
    );
}

/// `Lighting.Entry.ENTITY_IN_UI` (`setupForEntityInInventory`), which the
/// enchanting screen's book is lit with.
fn entity_in_ui_lights() -> (Vec3, Vec3) {
    (
        Vec3::new(0.2, -1.0, -1.0).normalize(),
        Vec3::new(-0.2, -1.0, 0.0).normalize(),
    )
}

/// `PictureInPictureRenderer.prepare` for the enchanting screen's 38x31 book
/// box at `(x0, y0)` (`translate(w / 2, 17)`, `scale(40, 40, -40)`), then
/// `GuiBookModelRenderer.renderToTexture`'s turns for `open`.
pub fn enchanting_book_pose(x0: f32, y0: f32, open: f32) -> Mat4 {
    let closed = 1.0 - open;
    Mat4::from_translation(Vec3::new(x0 + 19.0, y0 + 17.0, 0.0))
        * Mat4::from_scale(Vec3::new(40.0, 40.0, -40.0))
        * Mat4::from_rotation_y(PI)
        * Mat4::from_rotation_x(25f32.to_radians())
        * Mat4::from_translation(Vec3::new(closed * 0.2, closed * 0.1, closed * 0.25))
        * Mat4::from_rotation_y((-closed * 90.0 - 90.0).to_radians())
        * Mat4::from_rotation_x(PI)
}

/// `GuiBookModelRenderer.renderToTexture`'s page flips for `flip`: each
/// `clamp(frac(flip + offset) * 1.6 - 0.3, 0, 1)`.
pub fn enchanting_book_pages(flip: f32) -> (f32, f32) {
    let page = |offset: f32| {
        let x = flip + offset;
        ((x - x.floor()) * 1.6 - 0.3).clamp(0.0, 1.0)
    };
    (page(0.25), page(0.75))
}

/// `GuiGraphicsExtractor.book` from `EnchantmentScreen.extractBook`: the
/// enchanting table's book, `open` (0 to 1) and turning its pages with
/// `flip`, in the 38x31 box at `(x0, y0)`.
pub fn draw_enchanting_book(
    renderer: &ItemRenderer,
    sprites: &HashMap<String, Rect>,
    batch: &mut HudBatch,
    x0: f32,
    y0: f32,
    open: f32,
    flip: f32,
) {
    let (page1, page2) = enchanting_book_pages(flip);
    let mut quads = Vec::new();
    super::special::submit_book(
        renderer,
        enchanting_book_pose(x0, y0, open),
        open,
        page1,
        page2,
        sprites,
        &mut quads,
    );
    draw_quads(
        &quads,
        entity_in_ui_lights(),
        batch,
        [x0, y0, x0 + 38.0, y0 + 31.0],
        None,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enchanting_book_pages_turn_with_the_flip() {
        let close = |flip: f32, want: (f32, f32)| {
            let (a, b) = enchanting_book_pages(flip);
            assert!(
                (a - want.0).abs() < 1e-5 && (b - want.1).abs() < 1e-5,
                "{flip}: {a} {b}, want {want:?}"
            );
        };
        close(0.0, (0.1, 0.9));
        // A whole page further, the same pages.
        close(1.0, (0.1, 0.9));
        close(0.5, (0.9, 0.1));
        // The pages rest flat at either end of a turn.
        close(-0.25, (0.0, 0.5));
        close(0.6875, (1.0, 0.4));
    }
    #[test]
    fn a_closed_book_turns_away_and_an_open_one_faces_the_screen() {
        // The box centre maps the model origin; the book's origin moves with
        // `open` (`translate((1 - open) * (0.2, 0.1, 0.25))` before the turn).
        let open = enchanting_book_pose(0.0, 0.0, 1.0).transform_point3(Vec3::ZERO);
        assert!(
            (open - Vec3::new(19.0, 17.0, 0.0)).length() < 1e-4,
            "{open}"
        );
        let closed = enchanting_book_pose(0.0, 0.0, 0.0).transform_point3(Vec3::ZERO);
        assert!((closed - open).length() > 1.0, "{closed}");
    }

    fn face(y0: f32, y1: f32, z: impl Fn(f32) -> f32, color: u32) -> ItemQuad {
        ItemQuad {
            pos: [(0.0, y0), (0.0, y1), (8.0, y1), (8.0, y0)].map(|(x, y)| Vec3::new(x, y, z(y))),
            uv: [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]],
            foil_uv: [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]],
            gui_glint_uv: None,
            normal: Vec3::Z,
            color,
            foil: Foil::None,
            light_emission: 0,
            cull_back: false,
            translucent: false,
            oit: false,
        }
    }

    // Sample the emitted triangles in draw order, using opaque test faces.
    fn color_at(batch: &HudBatch, p: Vec2) -> Option<u32> {
        batch
            .vertices
            .as_chunks::<3>()
            .0
            .iter()
            .rev()
            .find_map(|tri| {
                let [a, b, c] = std::array::from_fn(|i| Vec2::from(tri[i].pos));
                let area = (b - a).perp_dot(c - a);
                if area.abs() < 1e-6 {
                    return None;
                }
                let u = (b - p).perp_dot(c - p) / area;
                let v = (c - p).perp_dot(a - p) / area;
                let w = 1.0 - u - v;
                (u >= 0.0 && v >= 0.0 && w >= 0.0).then_some(tri[0].color)
            })
    }

    #[test]
    fn banner_cloth_occludes_the_support_at_the_top_edge() {
        // The cloth is nearer where it overlaps the bar, although its
        // average depth is farther away because it slopes along its height.
        let bar = face(0.0, 1.0, |_| 0.0, 0xFF80_4000);
        let cloth = face(0.0, 8.0, |y| 2.0 - y, 0xFFFF_FFFF);
        let mut batch = HudBatch::scratch([320, 240]);
        draw_quads(
            &[bar, cloth],
            (Vec3::Z, Vec3::Z),
            &mut batch,
            [0.0, 0.0, 16.0, 16.0],
            None,
        );
        assert_eq!(color_at(&batch, Vec2::new(4.0, 0.5)), Some(u32::MAX));
    }

    #[test]
    fn intersecting_faces_change_draw_order_at_their_intersection() {
        let flat = face(0.0, 8.0, |_| 0.0, 0xFFFF_0000);
        let tilted = face(0.0, 8.0, |y| y - 4.0, 0xFF00_00FF);
        for quads in [[flat, tilted], [tilted, flat]] {
            let mut batch = HudBatch::scratch([320, 240]);
            draw_quads(
                &quads,
                (Vec3::Z, Vec3::Z),
                &mut batch,
                [0.0, 0.0, 16.0, 16.0],
                None,
            );
            assert_eq!(
                color_at(&batch, Vec2::new(4.0, 2.0)),
                Some(lit_color(&flat, (Vec3::Z, Vec3::Z)))
            );
            assert_eq!(
                color_at(&batch, Vec2::new(4.0, 6.0)),
                Some(lit_color(&tilted, (Vec3::Z, Vec3::Z)))
            );
            // The cut must interpolate the texture along with the position.
            for v in &batch.vertices {
                assert!((v.uv[0] - v.pos[0] / 8.0).abs() < 1e-6);
                assert!((v.uv[1] - v.pos[1] / 8.0).abs() < 1e-6);
            }
        }
    }

    #[test]
    fn coplanar_pattern_layers_keep_submission_order() {
        let base = face(0.0, 8.0, |y| 2.0 - y, 0xFFFF_FFFF);
        let pattern = ItemQuad {
            color: 0xFF00_00FF,
            ..base
        };
        let quads = [face(0.0, 1.0, |_| 0.0, 0xFF80_4000), base, pattern];
        let mut batch = HudBatch::scratch([320, 240]);
        draw_quads(
            &quads,
            (Vec3::Z, Vec3::Z),
            &mut batch,
            [0.0, 0.0, 16.0, 16.0],
            None,
        );
        for y in [0.5, 6.0] {
            assert_eq!(
                color_at(&batch, Vec2::new(4.0, y)),
                Some(lit_color(&quads[2], (Vec3::Z, Vec3::Z)))
            );
        }
    }

    #[test]
    fn cit_layers_draw_once_over_the_whole_item() {
        use crate::cit_glint::Global;
        use crate::hud::Blend;
        use crate::hud::cit_glint::Slot;
        use horizon_optifine::Properties;
        let rule = |name: &str, text: &str| {
            let path = format!("assets/minecraft/optifine/cit/{name}.properties");
            let text = format!("type=enchantment\ntexture=glint\n{text}");
            Rule::parse(&path, &Properties::parse_equals_first(&text), |_| true).unwrap()
        };
        let rules = [
            rule("shown", ""),
            rule("fire", "layer=1\nenchantments=fire_aspect"),
        ];
        // Shared by level, and the stack has no Fire Aspect: that layer is at intensity 0.
        let global = Global::parse(&Properties::parse_equals_first("method=average"));
        let glint = StackGlint::new(&rules, &[0, 1], &[("minecraft:sharpness", 3)], &global);
        assert_eq!(glint.layers.len(), 2);
        let slot = |blend| Slot {
            rgba: [1.0; 4],
            blur: false,
            blend,
        };
        let textures = GlintTextures {
            width: 1,
            height: 1,
            slot_of: vec![Some(1), Some(0)],
            slots: vec![slot(0), slot(1)],
            ..GlintTextures::default()
        };
        let cit = CitDraw {
            glint: &glint,
            textures: &textures,
            rules: &rules,
            millis: 0.0,
        };
        let mut batch = HudBatch::scratch([320, 240]);
        batch.glint_alpha = 0.5;
        let mut quad = face(0.0, 8.0, |_| 0.0, 0xFFFF_FFFF);
        quad.gui_glint_uv = Some([[42.0, 42.0]; 4]);
        draw_quads_with(
            &[quad],
            (Vec3::Z, Vec3::Z),
            &mut batch,
            [0.0, 0.0, 16.0, 16.0],
            None,
            Some(cit),
            None,
        );
        // The square's two triangles, then the one layer over both: six vertices a triangle.
        assert_eq!(batch.vertices.len(), 24);
        assert_eq!(
            batch.runs(),
            [(0..12, Blend::Alpha), (12..24, Blend::CitGlint(1))]
        );
        for v in &batch.vertices[12..] {
            // Slot 1 in red; full intensity at half the glint strength in alpha.
            assert_eq!(v.color.to_le_bytes(), [1, 0, 0, 128]);
            assert_eq!(v.glint_uv, v.pos, "CIT retains the sprite-local UVs");
        }
    }

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
            z: 0.0,
            uv: [x, y],
            glint_uv: [0.0, 0.0],
            cit_uv: [0.0, 0.0],
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

    fn test_item_renderer() -> ItemRenderer {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("items.json"), "{}").unwrap();
        let library = crate::model_library::ParsedModels::load(
            &crate::resource_source::ResourceSource::directory(root.to_path_buf()),
        )
        .into_item_library();
        let data = crate::test_resources::item_data(
            root,
            horizon_world::native_load_context::DataLoadOrigin::ClientPreload,
        )
        .unwrap();
        ItemRenderer::load_with_library(
            &horizon_assets::resource_source::ResourceRoot::directory(root.to_path_buf()),
            library,
            &data,
        )
        .unwrap()
    }

    fn test_baked_quad_model(
        sprite: &str,
        tint_index: i32,
        z: f32,
    ) -> Arc<horizon_assets::item_bake::BakedItemModel> {
        use horizon_assets::{
            bake::ModelQuad,
            item_bake::BakedItemModel,
            model::{Direction, Material},
        };
        let material = Material {
            sprite: sprite.to_owned(),
            force_translucent: false,
        };
        let quad = ModelQuad {
            positions: [
                [-0.5, -0.5, z],
                [0.5, -0.5, z],
                [0.5, 0.5, z],
                [-0.5, 0.5, z],
            ],
            uv: [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            facing: Direction::South,
            cull: None,
            material: material.clone(),
            tint_index,
            shade_direction_override: None,
            light_emission: 0,
            light: [0, 0],
            color: None,
            ambient_occlusion: None,
            render_type: None,
            shade: false,
        };
        Arc::new(BakedItemModel {
            quads: vec![quad],
            uses_block_light: false,
            particle: material,
            transforms: [const { None }; 9],
        })
    }

    fn test_layer(
        baked: Arc<horizon_assets::item_bake::BakedItemModel>,
        tints: Vec<i32>,
        matrix: Mat4,
        uses_block_light: bool,
    ) -> super::super::LayerRender {
        super::super::LayerRender {
            geometry: super::super::LayerGeometry::Quads(baked),
            tints,
            foil: super::super::FoilType::None,
            matrix,
            particle: String::new(),
            uses_block_light,
            extents: Arc::new(Vec::new()),
        }
    }

    fn test_state(layers: Vec<super::super::LayerRender>) -> ItemRenderState {
        ItemRenderState {
            display: horizon_assets::item_model::DisplayContext::Gui,
            layers,
            oversized_in_gui: false,
            animated: false,
            cit_glint: None,
            hdr_tier: 0,
            hdr_light: 0,
        }
    }

    fn hash_of(value: &impl std::hash::Hash) -> u64 {
        use std::hash::Hasher;
        let mut h = std::collections::hash_map::DefaultHasher::new();
        value.hash(&mut h);
        h.finish()
    }

    #[test]
    fn drawn_key_equality_hashing_and_bypass_cover_multi_layer_items() {
        let renderer = test_item_renderer();
        let sprites = HashMap::new();
        let baked_a = test_baked_quad_model("minecraft:item/apple", 0, 0.0);
        let baked_b = test_baked_quad_model("minecraft:item/stick", 1, 0.1);
        let baked_c = test_baked_quad_model("minecraft:item/bow", 2, 0.2);

        // DrawnTints equality, hash consistency, and length/value sensitivity across 0..=3+ tints.
        assert_eq!(DrawnTints::of(&[]), DrawnTints::of(&[]));
        assert_ne!(DrawnTints::of(&[]), DrawnTints::of(&[0]));
        assert_ne!(DrawnTints::of(&[0]), DrawnTints::of(&[0, 0]));
        assert_eq!(DrawnTints::of(&[1, 2]), DrawnTints::of(&[1, 2]));
        assert_ne!(DrawnTints::of(&[1, 2]), DrawnTints::of(&[1, 3]));
        assert_ne!(DrawnTints::of(&[1, 2]), DrawnTints::of(&[1, 2, 0]));
        assert_eq!(DrawnTints::of(&[1, 2, 3]), DrawnTints::of(&[1, 2, 3]));
        assert_ne!(DrawnTints::of(&[1, 2, 3]), DrawnTints::of(&[1, 2, 4]));
        for tints in [&[][..], &[1], &[1, 2], &[1, 2, 3]] {
            let a = DrawnTints::of(tints);
            let b = DrawnTints::of(tints);
            assert_eq!(a, b);
            assert_eq!(hash_of(&a), hash_of(&b));
        }

        // 0-, 1-, 2-, and 3-layer states all produce distinct, repeatable keys.
        let s0 = test_state(Vec::new());
        let s1 = test_state(vec![test_layer(
            baked_a.clone(),
            vec![0x112233],
            Mat4::IDENTITY,
            false,
        )]);
        let s2 = test_state(vec![
            test_layer(
                baked_a.clone(),
                vec![0x112233, 0x445566],
                Mat4::IDENTITY,
                false,
            ),
            test_layer(baked_b.clone(), vec![], Mat4::IDENTITY, true),
        ]);
        let s3 = test_state(vec![
            test_layer(baked_a.clone(), vec![1, 2, 3], Mat4::IDENTITY, false),
            test_layer(baked_b.clone(), vec![4], Mat4::IDENTITY, false),
            test_layer(baked_c.clone(), vec![], Mat4::IDENTITY, true),
        ]);
        let k0 = DrawnKey::of(&s0, &renderer, &sprites, 10.0, 20.0).unwrap();
        let k1 = DrawnKey::of(&s1, &renderer, &sprites, 10.0, 20.0).unwrap();
        let k2 = DrawnKey::of(&s2, &renderer, &sprites, 10.0, 20.0).unwrap();
        let k3 = DrawnKey::of(&s3, &renderer, &sprites, 10.0, 20.0).unwrap();

        let keys = [&k0, &k1, &k2, &k3];
        for (i, left) in keys.iter().enumerate() {
            for right in &keys[i + 1..] {
                assert_ne!(left, right);
            }
        }

        // Recomputing the key produces identical equality and hash.
        for (state, key) in [(&s0, &k0), (&s1, &k1), (&s2, &k2), (&s3, &k3)] {
            let again = DrawnKey::of(state, &renderer, &sprites, 10.0, 20.0).unwrap();
            assert_eq!(*key, again);
            assert_eq!(hash_of(key), hash_of(&again));
        }

        // Position, oversized flag, tints (single- and multi-tint), lighting, and transform
        // all differentiate keys.
        assert_ne!(
            k1,
            DrawnKey::of(&s1, &renderer, &sprites, 11.0, 20.0).unwrap()
        );
        let mut s1_oversized = s1.clone();
        s1_oversized.oversized_in_gui = true;
        assert_ne!(
            k1,
            DrawnKey::of(&s1_oversized, &renderer, &sprites, 10.0, 20.0).unwrap()
        );
        let mut s1_tinted = s1.clone();
        s1_tinted.layers[0].tints = vec![0x998877];
        assert_ne!(
            k1,
            DrawnKey::of(&s1_tinted, &renderer, &sprites, 10.0, 20.0).unwrap()
        );
        let mut s3_tinted = s3.clone();
        s3_tinted.layers[0].tints = vec![1, 2, 99];
        assert_ne!(
            k3,
            DrawnKey::of(&s3_tinted, &renderer, &sprites, 10.0, 20.0).unwrap()
        );
        let mut s1_lit = s1.clone();
        s1_lit.layers[0].uses_block_light = true;
        assert_ne!(
            k1,
            DrawnKey::of(&s1_lit, &renderer, &sprites, 10.0, 20.0).unwrap()
        );
        let mut s1_moved = s1.clone();
        s1_moved.layers[0].matrix = Mat4::from_translation(Vec3::X);
        assert_ne!(
            k1,
            DrawnKey::of(&s1_moved, &renderer, &sprites, 10.0, 20.0).unwrap()
        );

        // Dynamic / animated / foil / special items return None (bypass cache).
        let mut animated = s1.clone();
        animated.animated = true;
        assert!(DrawnKey::of(&animated, &renderer, &sprites, 10.0, 20.0).is_none());
        let mut foiled = s1.clone();
        foiled.layers[0].foil = super::super::FoilType::Standard;
        assert!(DrawnKey::of(&foiled, &renderer, &sprites, 10.0, 20.0).is_none());
        let mut special = s1;
        special.layers[0].geometry = super::super::LayerGeometry::Special {
            model: horizon_assets::item_model::SpecialModel {
                kind: horizon_assets::item_model::SpecialKind::Shield,
            },
            argument: serde_json::Value::Null,
        };
        assert!(DrawnKey::of(&special, &renderer, &sprites, 10.0, 20.0).is_none());
    }

    #[test]
    fn gui_draw_cache_hit_and_miss_match_uncached_draw_bit_for_bit() {
        use crate::hud::HudVertex;
        let renderer = test_item_renderer();
        let mut sprites = HashMap::new();
        let rect = Rect {
            u0: 0.0,
            v0: 0.0,
            u1: 0.5,
            v1: 0.5,
            w: 16,
            h: 16,
        };
        sprites.insert("item/apple".to_owned(), rect);
        sprites.insert("item/stick".to_owned(), rect);
        sprites.insert("item/bow".to_owned(), rect);

        let baked_a = test_baked_quad_model("minecraft:item/apple", 0, 0.0);
        let baked_b = test_baked_quad_model("minecraft:item/stick", 1, 0.1);
        let baked_c = test_baked_quad_model("minecraft:item/bow", 2, 0.2);

        let states = [
            // 1 layer, 1 tint
            test_state(vec![test_layer(
                baked_a.clone(),
                vec![0xFF33_AA55_u32 as i32],
                Mat4::IDENTITY,
                false,
            )]),
            // 2 layers, 2 tints
            test_state(vec![
                test_layer(
                    baked_a.clone(),
                    vec![0xFF11_2233_u32 as i32, 0xFF44_5566_u32 as i32],
                    Mat4::IDENTITY,
                    false,
                ),
                test_layer(baked_b.clone(), vec![], Mat4::IDENTITY, false),
            ]),
            // 3 layers, 3 tints
            test_state(vec![
                test_layer(
                    baked_a.clone(),
                    vec![
                        0xFFAA_0000_u32 as i32,
                        0xFF00_AA00_u32 as i32,
                        0xFF00_00AA_u32 as i32,
                    ],
                    Mat4::IDENTITY,
                    false,
                ),
                test_layer(baked_b, vec![0xFFFF_FF00_u32 as i32], Mat4::IDENTITY, false),
                test_layer(baked_c, vec![], Mat4::IDENTITY, false),
            ]),
        ];

        for (idx, state) in states.iter().enumerate() {
            let x = 16.0 + idx as f32 * 20.0;
            let y = 32.0;
            let mut reference = HudBatch::scratch([320, 240]);
            draw_now(
                state,
                &renderer,
                &sprites,
                &super::super::NoHost,
                &mut reference,
                x,
                y,
                0.0,
                1.0,
                0.75,
                None,
            );
            assert!(!reference.vertices.is_empty());

            // Cache miss populates entry.
            let mut miss_batch = HudBatch::scratch([320, 240]);
            draw(
                state,
                &renderer,
                &sprites,
                &super::super::NoHost,
                &mut miss_batch,
                x,
                y,
                0.0,
                1.0,
                0.75,
            );
            assert_eq!(renderer.gui_drawn.lock().unwrap().len(), idx + 1);
            assert_eq!(
                bytemuck::cast_slice::<HudVertex, u8>(&miss_batch.vertices),
                bytemuck::cast_slice::<HudVertex, u8>(&reference.vertices),
            );

            // Cache hit serves exact vertices without adding a new cache entry.
            let mut hit_batch = HudBatch::scratch([320, 240]);
            draw(
                state,
                &renderer,
                &sprites,
                &super::super::NoHost,
                &mut hit_batch,
                x,
                y,
                0.0,
                1.0,
                0.75,
            );
            assert_eq!(renderer.gui_drawn.lock().unwrap().len(), idx + 1);
            assert_eq!(
                bytemuck::cast_slice::<HudVertex, u8>(&hit_batch.vertices),
                bytemuck::cast_slice::<HudVertex, u8>(&reference.vertices),
            );
        }

        // Changing tint misses the cache and produces the new tinted vertices.
        let mut retinted = states[0].clone();
        retinted.layers[0].tints = vec![0xFF00_00FF_u32 as i32];
        let mut retinted_batch = HudBatch::scratch([320, 240]);
        let mut retinted_ref = HudBatch::scratch([320, 240]);
        draw(
            &retinted,
            &renderer,
            &sprites,
            &super::super::NoHost,
            &mut retinted_batch,
            16.0,
            32.0,
            0.0,
            1.0,
            0.75,
        );
        draw_now(
            &retinted,
            &renderer,
            &sprites,
            &super::super::NoHost,
            &mut retinted_ref,
            16.0,
            32.0,
            0.0,
            1.0,
            0.75,
            None,
        );
        assert_eq!(renderer.gui_drawn.lock().unwrap().len(), 4);
        assert_eq!(
            bytemuck::cast_slice::<HudVertex, u8>(&retinted_batch.vertices),
            bytemuck::cast_slice::<HudVertex, u8>(&retinted_ref.vertices),
        );

        // Animated items bypass the cache completely.
        let mut animated = states[0].clone();
        animated.animated = true;
        let mut anim_batch = HudBatch::scratch([320, 240]);
        draw(
            &animated,
            &renderer,
            &sprites,
            &super::super::NoHost,
            &mut anim_batch,
            16.0,
            32.0,
            0.0,
            1.0,
            0.75,
        );
        assert_eq!(renderer.gui_drawn.lock().unwrap().len(), 4);
    }
}
