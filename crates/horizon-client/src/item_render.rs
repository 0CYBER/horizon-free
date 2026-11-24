//! Item rendering through the resolved item model pipeline: the
//! `ItemModelResolver` of `horizon-assets` fills an [`ItemRenderState`]
//! (`ItemStackRenderState`: layers of baked quads or special renderers, each
//! with its display transform, local transform, tints and foil), and
//! [`ItemRenderState::submit`] turns it into the quads `ItemFeatureRenderer`
//! and the special model renderers submit, in pose space. A sink then draws
//! them: [`gui`] into the HUD batch with the GUI item lighting, [`WorldSink`]
//! into entity vertices with the level's lighting.
use horizon_assets::resource_source::{ResourceRoot, ResourceSource};
pub mod cit;
mod glint_atlas;
#[cfg(test)]
mod glint_tests;
pub mod gui;
mod special;
#[cfg(test)]
mod special_parts_tests;
#[cfg(test)]
mod tint_tests;

use crate::{
    block_entity_render::{ModelPart, diffuse},
    hud::Rect,
    mesher::{CULL_BACK, Vertex},
};
use glam::{Mat3, Mat4, Vec3};
use horizon_assets::{
    animation::AnimationMetadata,
    item_bake::{BakedItemModel, SpriteLookup, SpriteMask, bake_item_model, bake_sprite_layer},
    item_model::{
        ClientItem, Context, DisplayContext, FoilType, ItemEnv, ItemModelNode, ItemModels,
        LayerContent, LevelState, OwnerState, PreparedParts, SpecialModel, StackView,
    },
    model::Direction,
    palette::PalettedPermutations,
    resolve::{MISSING_MODEL, ModelLibrary},
};
use horizon_core::nbt::Tag;
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex, Weak},
};

/// One quad as submitted for drawing: pose-space corners, atlas UVs, the
/// transformed unit normal, ARGB colour (tint, `-1` white) and its glint.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ItemQuad {
    pub pos: [Vec3; 4],
    pub uv: [[f32; 2]; 4],
    /// The model UVs `GLINT_TEXTURING` scrolls for the glint (vanilla's UV0,
    /// or UV3 for a special model).
    pub foil_uv: [[f32; 2]; 4],
    /// Standard GUI glint uses the vanilla items atlas; CIT keeps sprite-local UVs.
    pub gui_glint_uv: Option<[[f32; 2]; 4]>,
    pub normal: Vec3,
    pub color: u32,
    pub foil: Foil,
    /// `MaterialInfo.lightEmission` (block light floor).
    pub light_emission: u8,
    /// The render type culls back faces (`entitySolid`): world drawing discards them.
    pub cull_back: bool,
    /// The quad's sprite has translucent pixels (`MaterialInfo.itemRenderType`
    /// is a translucent sheet): drawn with the translucent entities.
    pub translucent: bool,
    /// The quad's render type has an `OitPipelineSet` (`entityTranslucent`: a
    /// player head's skin): improved transparency draws it in its stages.
    pub oit: bool,
}
/// `TextureTransform.setupGlintTexturing`'s inputs.
#[derive(Clone, Copy, Debug)]
pub struct Glint {
    /// `options.glintSpeed` (vanilla default 0.5).
    pub speed: f32,
    /// `options.glintStrength` / shader `GlintAlpha` (vanilla default 0.75).
    pub alpha: f32,
    /// `Util.getMillis()`.
    pub millis: f32,
    /// The transform's scale: `GLINT_TEXTURING`'s 8.0 for GUI items,
    /// `ENTITY_GLINT_TEXTURING`'s 0.5 for items in the world and
    /// `ARMOR_ENTITY_GLINT_TEXTURING`'s 0.16 for armour (entity.wgsl).
    pub scale: f32,
}
impl Glint {
    /// `Util.getMillis() * glintSpeed * 8.0` as vanilla's long.
    fn ticks(millis: f32, speed: f32) -> i64 {
        (f64::from(millis) * f64::from(speed) * 8.0) as i64
    }
    /// The scroll clock modulo the common period of both offsets (110000 and
    /// 30000 divide 330000), for `globals.time.w`: the entity shader derives
    /// `ARMOR_ENTITY_GLINT_TEXTURING` from it.
    pub fn scroll_ticks(millis: f32, speed: f32) -> f32 {
        (Self::ticks(millis, speed) % 330_000) as f32
    }
    /// `TextureTransform.setupGlintTexturing(scale)` applied to the model UV:
    /// translation(-o0, o1) * rotateZ(PI/18) * scale. The result repeats:
    /// the shader samples the dedicated glint texture with vanilla's
    /// repeating `GlintSampler`, so the UVs stay unwrapped.
    pub fn uv(&self, model: [f32; 2]) -> [f32; 2] {
        let ticks = Self::ticks(self.millis, self.speed);
        let offset0 = (ticks % 110_000) as f32 / 110_000.0;
        let offset1 = (ticks % 30_000) as f32 / 30_000.0;
        let angle = std::f32::consts::PI / 18.0;
        let (sin, cos) = angle.sin_cos();
        [
            model[0] * self.scale * cos - model[1] * self.scale * sin - offset0,
            model[0] * self.scale * sin + model[1] * self.scale * cos + offset1,
        ]
    }
}
/// The glint inputs of one frame, read once: `GameRenderer.extract` snapshots
/// `glintSpeed` and `glintStrength` into `OptionsRenderState`
/// (`extractOptions`), and `TextureTransform.setupGlintTexturing` reads
/// `Util.getMillis()` once per glint render-type batch, not per item.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GlintFrame {
    /// `Util.getMillis()` this frame.
    pub millis: f32,
    /// `OptionsRenderState.glintSpeed`.
    pub speed: f32,
    /// `OptionsRenderState.glintStrength`.
    pub alpha: f32,
    /// The atlas has `misc/enchanted_glint_item`.
    pub sprite: bool,
}
impl GlintFrame {
    /// The glint at a texturing scale, if the atlas has its sprite.
    pub fn glint(&self, scale: f32) -> Option<Glint> {
        self.sprite.then_some(Glint {
            speed: self.speed,
            alpha: self.alpha,
            millis: self.millis,
            scale,
        })
    }
    /// [`Glint::scroll_ticks`] for this frame.
    pub fn scroll_ticks(&self) -> f32 {
        Glint::scroll_ticks(self.millis, self.speed)
    }
}
/// Which glint render type a quad uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Foil {
    None,
    /// `GLINT_TEXTURING` over the sprite's UVs (`itemGlintRenderType`,
    /// `entitySolidGlint`).
    Standard,
    /// `itemGlintSpecialRenderType`: compasses and clocks, a sheeted decal.
    Special,
}
/// Where submitted quads go.
pub trait ItemSink {
    fn quad(&mut self, quad: ItemQuad);
}
impl ItemSink for Vec<ItemQuad> {
    fn quad(&mut self, quad: ItemQuad) {
        self.push(quad);
    }
}

/// Data special renderers read from the game (registries, skins).
pub trait SpecialHost {
    /// The banner atlas texture of a banner pattern (`entity/banner/<asset>`).
    fn banner_texture(&self, pattern: &str) -> Option<String>;
    /// The shield atlas texture of a banner pattern (`entity/shield/<asset>`).
    fn shield_texture(&self, pattern: &str) -> Option<String>;
    /// The skin texture of a `minecraft:profile` component, if loaded.
    fn player_skin(&self, _profile: &Value) -> Option<String> {
        None
    }
}
/// A host with no registries: patterns and skins fall back to defaults.
pub struct NoHost;
impl SpecialHost for NoHost {
    fn banner_texture(&self, _: &str) -> Option<String> {
        None
    }
    fn shield_texture(&self, _: &str) -> Option<String> {
        None
    }
}

/// The geometry of one layer.
#[derive(Clone, Debug)]
pub enum LayerGeometry {
    Quads(Arc<BakedItemModel>),
    Special {
        model: SpecialModel,
        argument: Value,
    },
}
/// `ItemStackRenderState.LayerRenderState`.
#[derive(Clone, Debug)]
pub struct LayerRender {
    pub geometry: LayerGeometry,
    pub tints: Vec<i32>,
    pub foil: FoilType,
    /// `applyTransform`: the display transform, then the local transform.
    pub matrix: Mat4,
    /// `particleMaterial` (atlas sprite name).
    pub particle: String,
    pub uses_block_light: bool,
    /// Untransformed extents (quad corners or the special model's parts).
    pub extents: Arc<Vec<Vec3>>,
}
/// `ItemStackRenderState`.
#[derive(Clone, Debug)]
pub struct ItemRenderState {
    pub display: DisplayContext,
    pub layers: Vec<LayerRender>,
    pub oversized_in_gui: bool,
    pub animated: bool,
    /// The OptiFine CIT glint layers the stack draws in the GUI (`cit_glint`).
    pub cit_glint: Option<Arc<crate::cit_glint::StackGlint>>,
    /// HDR output's tier of an item that glows (`dynamic_light::Luminance::hdr_tier`).
    pub hdr_tier: u8,
    /// In HDR output, the block light of an item that glows: its block's, as a placed one has
    /// it, so a torch doesn't go dark in the shade while it shines. 0 for the rest.
    pub hdr_light: u8,
}
impl ItemRenderState {
    pub fn is_empty(&self) -> bool {
        self.layers.is_empty()
    }
    /// `usesBlockLight` of the first layer (GUI lighting choice).
    pub fn uses_block_light(&self) -> bool {
        self.layers.first().is_some_and(|l| l.uses_block_light)
    }
    /// `getModelBoundingBox`: the transformed extents of every layer, as
    /// (min, max); a zero box at the origin when there are none.
    pub fn bounding_box(&self) -> (Vec3, Vec3) {
        let mut lo = Vec3::splat(f32::INFINITY);
        let mut hi = Vec3::splat(f32::NEG_INFINITY);
        for layer in &self.layers {
            for p in layer.extents.iter() {
                let q = layer.matrix.transform_point3(*p);
                lo = lo.min(q);
                hi = hi.max(q);
            }
        }
        if lo.x > hi.x {
            (Vec3::ZERO, Vec3::ZERO)
        } else {
            (lo, hi)
        }
    }
    /// Bounds of the actual submitted item geometry under each world pose.
    /// Used for dropped-item occlusion because its collision box omits the
    /// render bob, model size, and copies in a stacked item cluster.
    pub fn posed_bounds(&self, poses: &[Mat4]) -> Option<(Vec3, Vec3)> {
        let mut lo = Vec3::splat(f32::INFINITY);
        let mut hi = Vec3::splat(f32::NEG_INFINITY);
        for &pose in poses {
            for layer in &self.layers {
                let transform = pose * layer.matrix;
                for &point in layer.extents.iter() {
                    let point = transform.transform_point3(point);
                    if !point.is_finite() {
                        return None;
                    }
                    lo = lo.min(point);
                    hi = hi.max(point);
                }
            }
        }
        (lo.x <= hi.x && lo.y <= hi.y && lo.z <= hi.z).then_some((lo, hi))
    }
    /// `ItemStackRenderState.submit` under `pose`.
    pub fn submit(
        &self,
        renderer: &ItemRenderer,
        sprites: &HashMap<String, Rect>,
        host: &dyn SpecialHost,
        pose: Mat4,
        sink: &mut dyn ItemSink,
    ) {
        for layer in &self.layers {
            let m = pose * layer.matrix;
            match &layer.geometry {
                LayerGeometry::Quads(baked) => {
                    submit_quads(renderer, baked, layer, self.display, m, sprites, sink);
                }
                LayerGeometry::Special { model, argument } => special::submit(
                    renderer,
                    model,
                    argument,
                    layer.foil != FoilType::None,
                    m,
                    sprites,
                    host,
                    sink,
                ),
            }
        }
    }
    /// [`Self::submit`] into a [`WorldSink`] of `light`, `lights` and `tier`,
    /// posed on the GPU (`gfx::entity_skin`): the same vertices in the same
    /// order. False, with nothing drawn, when a layer cannot take that path: a
    /// special model, a glint, or a quad that glows or is translucent (another
    /// material).
    pub fn submit_skinned(
        &self,
        renderer: &ItemRenderer,
        sprites: &HashMap<String, Rect>,
        pose: Mat4,
        (light, lights, tier): (u32, Option<(Vec3, Vec3)>, u8),
        skin: &mut crate::mobs::skin::Skin,
    ) -> bool {
        use crate::gfx::entity_skin::{SkinJob, SkinPart};
        // The axes' order in a stored vertex (`SkinVertex::axis`).
        const AXES: [Direction; 6] = [
            Direction::East,
            Direction::Up,
            Direction::South,
            Direction::West,
            Direction::Down,
            Direction::North,
        ];
        for layer in &self.layers {
            let LayerGeometry::Quads(baked) = &layer.geometry else {
                return false;
            };
            if layer.foil != FoilType::None {
                return false;
            }
            let Some(stored) = skin.models.item(baked, skin.frame) else {
                return false;
            };
            let translucent = |run: &crate::mobs::skin::ItemRun| {
                let sprite = &baked.quads[run.quad].material.sprite;
                renderer.sprites.translucent(sprite_name(sprite))
            };
            if stored.runs.iter().any(translucent) {
                return false;
            }
        }
        for layer in &self.layers {
            let LayerGeometry::Quads(baked) = &layer.geometry else {
                continue;
            };
            let m = pose * layer.matrix;
            let mut normals = FaceNormals::new(m);
            let Some(stored) = skin.models.item(baked, skin.frame) else {
                continue;
            };
            for run in &stored.runs {
                // `submit_quads`' sprite and tint, once for the run's quads.
                let q = &baked.quads[run.quad];
                let Some(r) = sprites
                    .get(sprite_name(&q.material.sprite))
                    .or_else(|| sprites.get(crate::hud::MISSING_SPRITE))
                else {
                    continue;
                };
                let color = if run.tint_index >= 0 {
                    layer
                        .tints
                        .get(run.tint_index as usize)
                        .copied()
                        .unwrap_or(-1)
                } else {
                    -1
                };
                let colors = AXES.map(|d| shaded(argb(color), normals.face(d), lights));
                let frame = &mut *skin.frame;
                frame.jobs.push(SkinJob {
                    first: run.first,
                    count: run.count,
                    part: frame.parts.len() as u32,
                    dst: frame.vertices,
                    rect: [r.u0, r.v0, r.u1, r.v1],
                    light,
                    overlay: u32::from(tier) << 16,
                    mode: SkinJob::ITEM,
                    ..Default::default()
                });
                frame.parts.push(SkinPart::item(m, colors));
                frame.vertices += run.count;
            }
            let bounds = stored.bounds;
            skin.grow_bounds(m, bounds);
        }
        true
    }
}

/// The atlas name of a sprite identifier (`minecraft:item/apple` -> `item/apple`).
pub fn sprite_name(sprite: &str) -> &str {
    sprite.strip_prefix("minecraft:").unwrap_or(sprite)
}
/// `Direction.getUnitVec3f`.
pub fn direction_vector(d: Direction) -> Vec3 {
    match d {
        Direction::Down => Vec3::NEG_Y,
        Direction::Up => Vec3::Y,
        Direction::North => Vec3::NEG_Z,
        Direction::South => Vec3::Z,
        Direction::West => Vec3::NEG_X,
        Direction::East => Vec3::X,
    }
}
/// `PoseStack.Pose.transformNormal`: the normal matrix, renormalized.
pub fn transform_normal(m: Mat4, n: Vec3) -> Vec3 {
    FaceNormals::new(m).transform(n)
}
/// [`transform_normal`] of one pose for many quads: its normal matrix inverted once, and
/// the normal of each of the six facings worked out the first time a quad uses it.
pub struct FaceNormals {
    normal: Mat3,
    faces: [Option<Vec3>; 6],
}
impl FaceNormals {
    pub fn new(m: Mat4) -> Self {
        Self {
            normal: Mat3::from_mat4(m).inverse().transpose(),
            faces: [None; 6],
        }
    }
    fn transform(&self, n: Vec3) -> Vec3 {
        (self.normal * n).normalize_or_zero()
    }
    /// The pose's normal of a face pointing `d`.
    pub fn face(&mut self, d: Direction) -> Vec3 {
        let i = d as usize;
        if let Some(n) = self.faces[i] {
            return n;
        }
        let n = self.transform(direction_vector(d));
        self.faces[i] = Some(n);
        n
    }
}
/// An `ARGB` int as the `u32` colour of an [`ItemQuad`].
pub fn argb(color: i32) -> u32 {
    color as u32
}

/// `ItemFeatureRenderer.prepareMainSubmit` for one cuboid layer.
fn submit_quads(
    renderer: &ItemRenderer,
    baked: &BakedItemModel,
    layer: &LayerRender,
    display: DisplayContext,
    m: Mat4,
    sprites: &HashMap<String, Rect>,
    sink: &mut dyn ItemSink,
) {
    let foil = match layer.foil {
        FoilType::None => Foil::None,
        FoilType::Standard => Foil::Standard,
        FoilType::Special => Foil::Special,
    };
    let atlas = renderer
        .glint_atlas
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut normals = FaceNormals::new(m);
    // Quads of one layer mostly share a sprite: look its rect and its
    // translucency up once.
    let mut last: Option<(&str, bool)> = None;
    let mut last_rect: Option<(&str, Option<Rect>)> = None;
    // A glint in the GUI follows the quad's sprite in the glint atlas: looked up once a sprite too.
    let gui_glint = display == DisplayContext::Gui && foil == Foil::Standard;
    let mut last_glint: Option<(&str, Option<Rect>)> = None;
    for q in &baked.quads {
        let rect = match last_rect {
            Some((sprite, rect)) if sprite == q.material.sprite => rect,
            _ => {
                let rect = sprites
                    .get(sprite_name(&q.material.sprite))
                    .or_else(|| sprites.get(crate::hud::MISSING_SPRITE))
                    .copied();
                last_rect = Some((&q.material.sprite, rect));
                rect
            }
        };
        let Some(r) = rect else {
            continue;
        };
        // `getLayerColorSafe`: an untinted quad, or a tint index past the
        // tints, draws white.
        let color = if q.tint_index >= 0 {
            layer
                .tints
                .get(q.tint_index as usize)
                .copied()
                .unwrap_or(-1)
        } else {
            -1
        };
        let pos = q.positions.map(|p| m.transform_point3(Vec3::from(p)));
        let uv =
            q.uv.map(|[u, v]| [r.u0 + (r.u1 - r.u0) * u, r.v0 + (r.v1 - r.v0) * v]);
        let translucent = match last {
            Some((sprite, translucent)) if sprite == q.material.sprite => translucent,
            _ => {
                let translucent = renderer
                    .sprites
                    .translucent(sprite_name(&q.material.sprite));
                last = Some((&q.material.sprite, translucent));
                translucent
            }
        };
        // Every item render type culls back faces: `Sheets.solidBlockSheet`
        // (`entitySolid`), `cutoutBlockSheet` (`entityCutout`) and
        // `translucentItemSheet` (`entityTranslucentCull`). Drawing both sides
        // z-fights the coplanar pairs a two-sided model already carries (a
        // calibrated sculk sensor's tendrils and amethyst planes, #4856; big
        // dripleaf, #4855; the inside of a held hay block, #4801) and stacks a
        // jelly shell's partial alpha until it looks opaque (#4508).
        let translucent = translucent || q.material.force_translucent;
        sink.quad(ItemQuad {
            pos,
            uv,
            foil_uv: q.uv,
            gui_glint_uv: gui_glint
                .then(|| match last_glint {
                    Some((sprite, rect)) if sprite == q.material.sprite => rect,
                    _ => {
                        let rect = atlas.get(sprite_name(&q.material.sprite)).copied();
                        last_glint = Some((&q.material.sprite, rect));
                        rect
                    }
                })
                .flatten()
                .map(|r| {
                    q.uv.map(|[u, v]| [r.u0 + (r.u1 - r.u0) * u, r.v0 + (r.v1 - r.v0) * v])
                }),
            normal: normals.face(q.facing),
            color: argb(color),
            foil,
            light_emission: q.light_emission,
            cull_back: true,
            translucent,
            oit: false,
        });
    }
}

// ---------------------------------------------------------------------------
// Sprites for baking
// ---------------------------------------------------------------------------

/// The alpha masks of item and block sprites, read from the texture files
/// (every unique animation frame) and the items atlas's palette sprites.
struct FileSprites {
    root: ResourceSource,
    permutations: Vec<PalettedPermutations>,
    cache: Mutex<HashMap<String, Option<SpriteMask>>>,
    /// Sprites with a pixel that is neither fully transparent nor opaque
    /// (`Transparency.hasTranslucent`), filled in as sprites load.
    translucent: Mutex<HashMap<String, bool>>,
}
impl SpriteLookup for FileSprites {
    fn mask(&self, sprite: &str) -> Option<SpriteMask> {
        if let Ok(cache) = self.cache.lock()
            && let Some(hit) = cache.get(sprite)
        {
            return hit.clone();
        }
        let mask = self.load(sprite);
        if let Ok(mut cache) = self.cache.lock() {
            cache.insert(sprite.to_owned(), mask.clone());
        }
        mask
    }
}
fn argb_pixels(rgba: &[u8]) -> Vec<u32> {
    rgba.as_chunks::<4>()
        .0
        .iter()
        .map(|p| u32::from_be_bytes([p[3], p[0], p[1], p[2]]))
        .collect()
}
impl FileSprites {
    fn translucent(&self, sprite: &str) -> bool {
        let known = |this: &Self| this.translucent.lock().ok()?.get(sprite).copied();
        known(self).unwrap_or_else(|| {
            self.mask(sprite);
            known(self).unwrap_or(false)
        })
    }
    fn png(&self, sprite: &str, directory: &str) -> String {
        crate::atlas_sources::texture_resource(sprite, directory)
    }
    fn image(&self, sprite: &str, directory: &str) -> Option<(usize, usize, Vec<u32>)> {
        let (w, h, rgba) =
            crate::hud::decode_bytes(&self.root.read(&self.png(sprite, directory)).ok()?).ok()?;
        Some((w as usize, h as usize, argb_pixels(&rgba)))
    }
    fn load(&self, sprite: &str) -> Option<SpriteMask> {
        let png = self.png(sprite, "textures");
        let (iw, ih, argb) = match self.image(sprite, "textures") {
            Some(image) => image,
            None => {
                let p = self.permutations.iter().find_map(|s| s.find(sprite))?;
                let (w, h, mut pixels) = self.image(p.base, "textures")?;
                let (_, _, key) = self.image(p.palette_key, "textures/palettes")?;
                let (_, _, target) = self.image(p.palette, "textures/palettes")?;
                horizon_assets::palette::apply_palette(&mut pixels, &key, &target)?;
                (w, h, pixels)
            }
        };
        let translucent = argb.iter().any(|p| matches!(p >> 24, 1..=254));
        if let Ok(mut cache) = self.translucent.lock() {
            cache.insert(sprite.to_owned(), translucent);
        }
        let alpha: Vec<u8> = argb.iter().map(|p| (p >> 24) as u8).collect();
        let animation = self
            .root
            .read_to_string(&format!("{png}.mcmeta"))
            .ok()
            .and_then(|m| serde_json::from_str::<Value>(&m).ok())
            .and_then(|m| m.get("animation").map(Value::to_string))
            .and_then(|a| AnimationMetadata::from_json(&a).ok());
        let (w, h) = match &animation {
            Some(a) => {
                let [w, h] = a.frame_size(iw as i32, ih as i32);
                (w as usize, h as usize)
            }
            None => (iw, ih),
        };
        if w == 0 || h == 0 {
            return None;
        }
        let columns = iw / w;
        let total = columns * (ih / h);
        let clip = animation.and_then(|a| a.clip(total as u32).ok().flatten());
        let frames: Vec<usize> = match clip {
            Some(clip) => {
                let mut unique = Vec::new();
                for (index, _) in clip.sequence() {
                    if !unique.contains(&(*index as usize)) {
                        unique.push(*index as usize);
                    }
                }
                unique
            }
            None => vec![0],
        };
        let frames = frames
            .into_iter()
            .map(|f| {
                let (fx, fy) = (f % columns * w, f / columns * h);
                (0..h)
                    .flat_map(|y| (0..w).map(move |x| (x, y)))
                    .map(|(x, y)| alpha[(fy + y) * iw + fx + x] == 0)
                    .collect()
            })
            .collect();
        Some(SpriteMask {
            width: w,
            height: h,
            frames,
        })
    }
}

// ---------------------------------------------------------------------------
// The renderer
// ---------------------------------------------------------------------------

/// The baked item models and everything resolution needs.
pub struct ItemRenderer {
    pub models: ItemModels,
    library: Arc<ModelLibrary>,
    sprites: FileSprites,
    baked: Mutex<HashMap<String, Arc<BakedItemModel>>>,
    extents: Mutex<HashMap<String, Arc<Vec<Vec3>>>>,
    /// GUI items already drawn: the HUD vertices of a static item at a GUI position
    /// (`gui::draw`), which are the same every frame it stays there.
    pub(crate) gui_drawn: Mutex<HashMap<gui::DrawnKey, Arc<[crate::hud::HudVertex]>>>,
    /// Entity model layers of the special renderers, with the head of each skull type a native
    /// mod registered, under its kind.
    parts: HashMap<String, ModelPart>,
    /// The default texture of each skull type a native mod registered (`SKIN_BY_TYPE`).
    skull_textures: HashMap<&'static str, &'static str>,
    /// `colormap/grass.png` as ARGB.
    grass: Vec<i32>,
    /// `minecraft:item` registry ids (`Item.getId`).
    item_ids: HashMap<String, i32>,
    /// The enabled packs' OptiFine CIT rules and their swapped bakes; `None`
    /// without rules.
    pub cit: Option<cit::CitModels>,
    glint_atlas: Mutex<HashMap<String, Rect>>,
}

/// Every `items/**/*.json` client item definition, by identifier. Through
/// Resolved resource names preserve nested item identifiers.
fn load_definitions(root: &ResourceSource) -> BTreeMap<String, ClientItem> {
    let mut out = BTreeMap::new();
    for namespace in root.namespaces("assets") {
        let prefix = format!("assets/{namespace}/items/");
        for name in root.list(&prefix) {
            let Some(id) = name
                .strip_prefix(&prefix)
                .and_then(|name| name.strip_suffix(".json"))
            else {
                continue;
            };
            let Ok(text) = root.read_to_string(&name) else {
                continue;
            };
            match ClientItem::from_json(&text) {
                Ok(item) => {
                    out.insert(format!("{namespace}:{id}"), item);
                }
                Err(error) => {
                    log::warn!("Horizon: item definition {namespace}:{id} not loaded: {error}")
                }
            }
        }
    }
    out
}

fn glint_padding(filtering: i32, bit: i32) -> usize {
    if filtering == 2 {
        bit.wrapping_sub(1).clamp(0, 4) as usize
    } else {
        0
    }
}

impl ItemRenderer {
    /// Prepare GUI glint after the device exists; propagate packing failures to pack rollback.
    pub fn prepare_glint_atlas(&self, maximum: u32, filtering: (i32, i32)) -> crate::Result<()> {
        let padding = glint_padding(filtering.0, filtering.1);
        let sprites = glint_atlas::load(&self.sprites.root, 1 << padding, maximum)?;
        *self
            .glint_atlas
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = sprites;
        Ok(())
    }

    /// Load item definitions, item prototypes (`items.json`), every model,
    /// the special renderers' entity models and the grass colormap.
    pub fn load(
        root: &ResourceRoot,
        item_data: &horizon_world::item_defaults::ItemData,
    ) -> crate::Result<Self> {
        Self::load_with_library(
            root,
            crate::model_library::ParsedModels::load(root.source()).into_item_library(),
            item_data,
        )
    }
    /// Reuse the immutable models prepared with the block resources at `root`.
    /// Defer item-only validation failures until after item definitions are loaded.
    pub fn load_with_library(
        root: &ResourceRoot,
        library: std::result::Result<Arc<ModelLibrary>, String>,
        item_data: &horizon_world::item_defaults::ItemData,
    ) -> crate::Result<Self> {
        Self::load_tables(root, library, true, item_data, None)
    }
    /// What a dedicated server reads: the item prototypes (the stacks'
    /// default components) and ids, without the client item definitions,
    /// the special renderers' entity models and the grass colormap, which
    /// only drawing reads (1.4 MB and ~1,500 files).
    pub fn load_for_server(
        root: &ResourceRoot,
        library: std::result::Result<Arc<ModelLibrary>, String>,
        item_data: &horizon_world::item_defaults::ItemData,
    ) -> crate::Result<Self> {
        Self::load_tables(root, library, false, item_data, None)
    }
    pub fn load_with_library_and_entity_models(
        root: &horizon_assets::resource_source::ResourceRoot,
        library: std::result::Result<Arc<ModelLibrary>, String>,
        document: &crate::mobs::EntityModelDocument,
        item_data: &horizon_world::item_defaults::ItemData,
    ) -> crate::Result<Self> {
        Self::load_tables(root, library, true, item_data, Some(document))
    }
    fn load_tables(
        root: &ResourceRoot,
        library: std::result::Result<Arc<ModelLibrary>, String>,
        drawn: bool,
        item_data: &horizon_world::item_defaults::ItemData,
        entity_models: Option<&crate::mobs::EntityModelDocument>,
    ) -> crate::Result<Self> {
        let prototypes = Arc::clone(item_data.prototypes()?);
        let models = ItemModels {
            definitions: if drawn {
                load_definitions(root.source())
            } else {
                BTreeMap::new()
            },
            prototypes,
        };
        // CIT models and glint textures only affect drawing. A server still
        // needs item prototypes and ids, but never these resource-pack assets.
        let (library, cit) = if drawn {
            cit::load(root.source(), library?)
        } else {
            (library?, None)
        };
        let permutations = PalettedPermutations::from_atlas(
            &root
                .source()
                .read_to_string("assets/minecraft/atlases/items.json")
                .unwrap_or_default(),
        );
        let mut parts = if drawn {
            entity_models.map_or_else(
                || special::load_parts(root.prepared()),
                special::load_parts_from_document,
            )
        } else {
            HashMap::new()
        };
        // Mod skull types, resolved once here so a head draws as a vanilla one does.
        let mut skull_textures = HashMap::new();
        if drawn {
            for (kind, part, texture) in crate::skull_models::all() {
                parts.insert(kind.to_owned(), part.clone());
                skull_textures.insert(kind, texture);
            }
        }
        let grass = if drawn {
            root.source()
                .read("assets/minecraft/textures/colormap/grass.png")
                .map_err(Into::into)
                .and_then(|bytes| crate::hud::decode_bytes(&bytes))
                .map(|(_, _, rgba)| argb_pixels(&rgba).into_iter().map(|p| p as i32).collect())
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let item_ids = horizon_protocol::registry::Builtin::find(root.prepared())
            .and_then(|p| horizon_protocol::registry::Builtin::load(&p).ok())
            .and_then(|b| {
                let ids = b.get("minecraft:item")?;
                let mut out = HashMap::new();
                for i in 0.. {
                    let Some(name) = ids.name(i) else { break };
                    out.insert(name.to_owned(), i);
                }
                Some(out)
            })
            .unwrap_or_default();
        Ok(Self {
            glint_atlas: Mutex::default(),
            item_ids,
            models,
            library,
            sprites: FileSprites {
                root: root.source().clone(),
                permutations,
                cache: Mutex::default(),
                translucent: Mutex::default(),
            },
            baked: Mutex::default(),
            extents: Mutex::default(),
            gui_drawn: Mutex::default(),
            parts,
            skull_textures,
            grass,
            cit,
        })
    }
    /// The models a pack's own item definitions (outside the `minecraft` namespace) draw, at any
    /// depth of their composite, condition, select and range nodes.
    pub fn pack_model_names(&self) -> Vec<String> {
        fn leaves(node: &ItemModelNode, out: &mut Vec<String>) {
            match node {
                ItemModelNode::Model(leaf) => out.push(leaf.model.clone()),
                ItemModelNode::Composite { models, .. } => {
                    models.iter().for_each(|m| leaves(m, out))
                }
                ItemModelNode::Condition {
                    on_true, on_false, ..
                } => {
                    leaves(on_true, out);
                    leaves(on_false, out);
                }
                ItemModelNode::Select {
                    cases, fallback, ..
                } => {
                    cases.iter().for_each(|c| leaves(&c.model, out));
                    fallback.iter().for_each(|f| leaves(f, out));
                }
                ItemModelNode::RangeDispatch {
                    entries, fallback, ..
                } => {
                    entries.iter().for_each(|(_, m)| leaves(m, out));
                    fallback.iter().for_each(|f| leaves(f, out));
                }
                ItemModelNode::Registered { models, .. } => {
                    models.iter().for_each(|(_, m)| leaves(m, out))
                }
                _ => {}
            }
        }
        let mut names = Vec::new();
        for (_, item) in self
            .models
            .definitions
            .iter()
            .filter(|(id, _)| !id.starts_with("minecraft:"))
        {
            leaves(&item.model, &mut names);
        }
        names.sort_unstable();
        names.dedup();
        names
    }
    /// Bakes `names` ahead, with their corners: vanilla bakes every model when it loads a pack, so
    /// the first time one shows nothing stops to bake it (a server's large models, its railway's,
    /// took seconds). Stops once the pack's renderer is dropped (the pack unloaded or reloaded).
    pub fn bake_ahead(renderer: &Weak<Self>, names: Vec<String>) {
        for name in names {
            let Some(renderer) = renderer.upgrade() else {
                return;
            };
            let baked = renderer.bake(&name);
            renderer.quad_extents(&name, &baked);
        }
    }
    /// `ModelBakery` item baking, cached per model.
    pub fn bake(&self, model: &str) -> Arc<BakedItemModel> {
        self.baked_with(model, || {
            bake_item_model(&self.library, &self.sprites, model)
        })
    }
    /// A layer baked from something other than a model id, such as an extruded sprite,
    /// cached under `key` (which no model id can equal: identifiers hold no NUL).
    fn baked_with(
        &self,
        key: &str,
        bake: impl FnOnce() -> std::result::Result<BakedItemModel, horizon_assets::DecodeError>,
    ) -> Arc<BakedItemModel> {
        if let Ok(cache) = self.baked.lock()
            && let Some(hit) = cache.get(key)
        {
            return hit.clone();
        }
        let baked = bake()
            .or_else(|_| bake_item_model(&self.library, &self.sprites, MISSING_MODEL))
            .map(Arc::new)
            .unwrap_or_else(|_| {
                Arc::new(BakedItemModel {
                    quads: Vec::new(),
                    uses_block_light: false,
                    particle: horizon_assets::model::Material {
                        sprite: horizon_assets::item_bake::MISSING_SPRITE.to_owned(),
                        force_translucent: false,
                    },
                    transforms: Default::default(),
                })
            });
        if let Ok(mut cache) = self.baked.lock() {
            cache.insert(key.to_owned(), baked.clone());
        }
        baked
    }
    /// `CuboidItemModelWrapper.computeExtents`: the distinct quad corners.
    fn quad_extents(&self, model: &str, baked: &BakedItemModel) -> Arc<Vec<Vec3>> {
        if let Ok(cache) = self.extents.lock()
            && let Some(hit) = cache.get(model)
        {
            return hit.clone();
        }
        let out = Arc::new(corners(baked));
        if let Ok(mut cache) = self.extents.lock() {
            cache.insert(model.to_owned(), out.clone());
        }
        out
    }
    fn special_extents(&self, model: &SpecialModel) -> Arc<Vec<Vec3>> {
        let key = model.to_json().to_string();
        if let Ok(cache) = self.extents.lock()
            && let Some(hit) = cache.get(&key)
        {
            return hit.clone();
        }
        let out = Arc::new(special::extents(self, model));
        if let Ok(mut cache) = self.extents.lock() {
            cache.insert(key, out.clone());
        }
        out
    }
    /// A special model renderer's answer `parts` for a model node of `base`, prepared to draw:
    /// `base` and each part's model baked, once, when the answer is accepted, so a draw looks
    /// nothing up by name. A part naming a model the packs don't have refuses them all, and the
    /// layer draws the missing model.
    pub fn prepare_parts(
        &self,
        base: &str,
        parts: &[horizon_assets::item_model::SpecialPart],
    ) -> PreparedParts {
        let part = |model: &str, tints, pose| {
            let geometry = self.bake(model);
            PreparedPart {
                extents: self.quad_extents(model, &geometry),
                geometry,
                tints,
                pose,
            }
        };
        let parts = match parts.iter().find(|p| !self.library.resolves(&p.model)) {
            Some(unknown) => {
                warn_unknown_part_once(&unknown.model);
                vec![part(
                    MISSING_MODEL,
                    Vec::new(),
                    horizon_assets::math::Mat4::IDENTITY,
                )]
            }
            None => parts
                .iter()
                .map(|p| part(&p.model, p.tints.clone(), p.pose))
                .collect(),
        };
        PreparedParts(Arc::new(PreparedSpecial {
            base: self.bake(base),
            parts,
        }))
    }
    /// The layers of a prepared answer (`LayerContent::Parts`), one for each part: the part's own
    /// model's quads, posed under the layer's transform, with the base model's display transform,
    /// particle and GUI light.
    fn push_parts(
        &self,
        layers: &mut Vec<LayerRender>,
        display: DisplayContext,
        (foil, transform): (FoilType, horizon_assets::math::Mat4),
        prepared: &PreparedParts,
    ) {
        let Some(prepared) = prepared.0.downcast_ref::<PreparedSpecial>() else {
            // Only `prepare_parts` makes them.
            return;
        };
        let base = &prepared.base;
        for part in &prepared.parts {
            // Under `applyTransform`'s half-block offset, so a part's pose is about the model's
            // corner, as its elements are.
            layers.push(LayerRender {
                geometry: LayerGeometry::Quads(part.geometry.clone()),
                tints: part.tints.clone(),
                foil,
                matrix: glam_matrix(&base.layer_matrix(display, &transform.mul(&part.pose))),
                uses_block_light: base.uses_block_light,
                particle: sprite_name(&base.particle.sprite).to_owned(),
                extents: part.extents.clone(),
            });
        }
    }
    /// `ItemModelResolver.updateForTopItem`.
    pub fn resolve(
        &self,
        stack: &StackView,
        display: DisplayContext,
        owner: Option<&OwnerState>,
        level: Option<&LevelState>,
        seed: i32,
        env: &dyn ItemEnv,
    ) -> ItemRenderState {
        self.resolve_with_rule(stack, display, owner, level, seed, env, None)
    }
    /// [`Self::resolve`] for a stack OptiFine CIT item rule `rule` (an index
    /// into `cit.rules.rules`) matched: its model layers draw what the rule
    /// swaps them for.
    #[allow(clippy::too_many_arguments)]
    pub fn resolve_with_rule(
        &self,
        stack: &StackView,
        display: DisplayContext,
        owner: Option<&OwnerState>,
        level: Option<&LevelState>,
        seed: i32,
        env: &dyn ItemEnv,
        rule: Option<usize>,
    ) -> ItemRenderState {
        let ctx = Context {
            display,
            owner,
            level,
            seed,
        };
        let resolved = self.models.resolve(stack, &ctx, env);
        let mut animated = false;
        let mut layers = Vec::with_capacity(resolved.layers.len());
        for layer in resolved.layers {
            let (geometry, baked, extents) = match layer.content {
                LayerContent::Model { model } => {
                    let (baked, extents) = rule
                        .and_then(|rule| self.cit_layer(rule, &model))
                        .unwrap_or_else(|| {
                            let baked = self.bake(&model);
                            let extents = self.quad_extents(&model, &baked);
                            (baked, extents)
                        });
                    (LayerGeometry::Quads(baked.clone()), baked, extents)
                }
                LayerContent::Sprite { sprite, mask } => {
                    let key = sprite_layer_key(&sprite, mask.as_deref());
                    let baked = self.baked_with(&key, || {
                        bake_sprite_layer(&self.library, &self.sprites, &sprite, mask.as_deref())
                    });
                    let extents = self.quad_extents(&key, &baked);
                    (LayerGeometry::Quads(baked.clone()), baked, extents)
                }
                LayerContent::Missing => {
                    let baked = self.bake(MISSING_MODEL);
                    let extents = self.quad_extents(MISSING_MODEL, &baked);
                    (LayerGeometry::Quads(baked.clone()), baked, extents)
                }
                LayerContent::Special {
                    base,
                    model,
                    argument,
                } => {
                    let baked = self.bake(&base);
                    let extents = self.special_extents(&model);
                    (LayerGeometry::Special { model, argument }, baked, extents)
                }
                LayerContent::Parts(prepared) => {
                    // What a mod drew may change with every draw, so no GUI item is kept.
                    animated = true;
                    self.push_parts(
                        &mut layers,
                        display,
                        (layer.foil, layer.transform),
                        &prepared,
                    );
                    continue;
                }
            };
            if layer.foil != FoilType::None {
                animated = true;
            }
            layers.push(LayerRender {
                geometry,
                tints: layer.tints,
                foil: layer.foil,
                matrix: glam_matrix(&baked.layer_matrix(display, &layer.transform)),
                uses_block_light: baked.uses_block_light,
                particle: sprite_name(&baked.particle.sprite).to_owned(),
                extents,
            });
        }
        ItemRenderState {
            display,
            layers,
            oversized_in_gui: resolved.oversized_in_gui,
            animated,
            cit_glint: None,
            hdr_tier: 0,
            hdr_light: 0,
        }
    }
    /// `GrassColor.get` over the loaded colormap.
    pub fn grass_color(&self, temperature: f64, downfall: f64) -> i32 {
        let rain = downfall * temperature;
        let x = ((1.0 - temperature) * 255.0) as i64;
        let y = ((1.0 - rain) * 255.0) as i64;
        let index = (y << 8 | x) as usize;
        self.grass.get(index).copied().unwrap_or(-65281)
    }
    /// Submit a special model renderer directly (worn skulls of
    /// `CustomHeadLayer`, which draw the skull model, not the item).
    #[allow(clippy::too_many_arguments)]
    pub fn submit_special(
        &self,
        model: &SpecialModel,
        argument: &Value,
        has_foil: bool,
        pose: Mat4,
        sprites: &HashMap<String, Rect>,
        host: &dyn SpecialHost,
        sink: &mut dyn ItemSink,
    ) {
        special::submit(self, model, argument, has_foil, pose, sprites, host, sink);
    }
    /// `Item.getId` (0 when the registry report is missing).
    pub fn item_id(&self, item: &str) -> i32 {
        self.item_ids.get(item).copied().unwrap_or(0)
    }
    pub(crate) fn part(&self, key: &str) -> Option<&ModelPart> {
        self.parts.get(key)
    }
}
/// A cache key for an extruded sprite layer: `\0` keeps it out of model ids' space.
fn sprite_layer_key(sprite: &str, mask: Option<&str>) -> String {
    format!("\0{sprite}\0{}", mask.unwrap_or_default())
}
/// `CuboidItemModelWrapper.computeExtents`: the distinct quad corners.
fn corners(baked: &BakedItemModel) -> Vec<Vec3> {
    let mut out: Vec<Vec3> = Vec::new();
    for q in &baked.quads {
        for p in q.positions {
            let p = Vec3::from(p);
            if !out.contains(&p) {
                out.push(p);
            }
        }
    }
    out
}
/// A special model renderer's answer as [`ItemRenderer::prepare_parts`] baked it.
struct PreparedSpecial {
    /// The model node's `base`: its display transforms, particle and GUI light.
    base: Arc<BakedItemModel>,
    parts: Vec<PreparedPart>,
}
struct PreparedPart {
    geometry: Arc<BakedItemModel>,
    extents: Arc<Vec<Vec3>>,
    tints: Vec<i32>,
    pose: horizon_assets::math::Mat4,
}
/// Says once per model id that a mod's special model renderer drew a model the packs don't have.
fn warn_unknown_part_once(model: &str) {
    static SEEN: Mutex<std::collections::BTreeSet<String>> =
        Mutex::new(std::collections::BTreeSet::new());
    if SEEN
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(model.to_owned())
    {
        log::warn!(
            "Horizon: a special model renderer drew the model {model}, which no pack has; the item draws the missing model"
        );
    }
}
/// A `horizon-assets` (row-major, JOML-exact) matrix as glam's.
pub fn glam_matrix(m: &horizon_assets::math::Mat4) -> Mat4 {
    Mat4::from_cols_array_2d(&m.rows()).transpose()
}

// ---------------------------------------------------------------------------
// Stacks
// ---------------------------------------------------------------------------

/// An NBT value as the JSON its component codec reads.
pub fn tag_json(tag: &Tag) -> Value {
    horizon_client_core::component_codec::raw_tag_json(tag)
}
/// The resolver's view of a client stack: its item, count and component patch.
pub fn stack_view(stack: &crate::items::ItemStack) -> StackView {
    let mut view = StackView::new(&stack.item, stack.count as i32);
    view.bundle_selected = stack.data().map_or(-1, |d| d.bundle_selected.0);
    if let Some(Tag::Compound(c)) = crate::item_data::components_tag(stack.damage, stack.data()) {
        for (k, v) in &c {
            let key = k.to_string_lossy();
            match key.strip_prefix('!') {
                Some(removed) => view.patch.insert(removed.to_owned(), None),
                None => {
                    let value =
                        horizon_client_core::component_codec::nbt_component_to_json(&key, v);
                    view.patch.insert(key, Some(value))
                }
            };
        }
    }
    view
}

// ---------------------------------------------------------------------------
// World drawing
// ---------------------------------------------------------------------------

/// Entity vertices for submitted quads: the `item`/`entity` shader's
/// `minecraft_mix_light` with the level's diffuse lights on the vertex
/// colour, the packed lightmap coordinates (raised by light emission) and
/// overlay 7 for the glint.
pub struct WorldSink<'a> {
    pub out: &'a mut Vec<Vertex>,
    /// Packed light (`block_entity_render::vertex_light`).
    pub light: u32,
    /// The level's diffuse lights; `None` leaves the colour unshaded.
    pub lights: Option<(Vec3, Vec3)>,
    /// The `GLINT_TEXTURING` sample for foil quads (`None` leaves them unglinted).
    pub glint: Option<Glint>,
    /// HDR output's tier (`ItemRenderState::hdr_tier`): rides in the layer's high bits.
    pub tier: u8,
}
/// The vertex colour [`WorldSink`] gives a quad of `color` (ARGB) facing `normal`:
/// shaded by `lights` (`None`: unshaded), as ABGR bytes.
fn shaded(color: u32, normal: Vec3, lights: Option<(Vec3, Vec3)>) -> u32 {
    let brightness = lights.map_or(1.0, |l| diffuse(normal, l));
    let [a, r, g, b] = color.to_be_bytes();
    let c = |v: u8| (f32::from(v) * brightness) as u32;
    u32::from(a) << 24 | c(b) << 16 | c(g) << 8 | c(r)
}
impl ItemSink for WorldSink<'_> {
    fn quad(&mut self, q: ItemQuad) {
        let color = shaded(q.color, q.normal, self.lights);
        // `LightCoordsUtil.lightCoordsWithEmission`: block light at least the emission.
        let emission = u32::from(q.light_emission) * 16;
        let light = if (self.light >> 8 & 0xFF) < emission {
            (self.light & 0xFF) | emission << 8
        } else {
            self.light
        };
        let glint = if q.foil == Foil::None {
            None
        } else {
            self.glint
        };
        let overlay = match (glint.is_some(), q.translucent) {
            (false, true) => crate::gfx::TRANSLUCENT_ITEM_OVERLAY,
            (false, false) => 0,
            (true, true) => crate::gfx::TRANSLUCENT_GLINT_ITEM_OVERLAY,
            (true, false) => 7,
        };
        // Only plain and foil quads: the translucent sheet's overlays are their own material.
        let oit = if q.oit && matches!(overlay, 0 | 7) {
            crate::gfx::OIT_ENTITY
        } else {
            0
        };
        for (i, (p, uv)) in q.pos.iter().zip(q.uv).enumerate() {
            let glint_uv = glint.map_or([0.0, 0.0], |glint| glint.uv(q.foil_uv[i]));
            self.out.push(Vertex {
                normal: if q.cull_back { CULL_BACK } else { [0; 4] },
                pos: p.to_array(),
                uv,
                layer: overlay | oit | u32::from(self.tier) << 16,
                color,
                light,
                glint_uv,
            });
        }
    }
}

/// `ItemEntity.getSpin`.
pub fn item_spin(age_in_ticks: f32, bob_offset: f32) -> f32 {
    age_in_ticks / 20.0 + bob_offset
}
/// `ItemClusterRenderState.getRenderedAmount`.
pub fn rendered_amount(count: u32) -> u32 {
    match count {
        0..=1 => 1,
        2..=16 => 2,
        17..=32 => 3,
        33..=48 => 4,
        _ => 5,
    }
}
/// `ItemEntityRenderer.submitMultipleFromCount`: the poses of each copy of
/// a stack (`seed` is `ItemClusterRenderState.getSeedForItemStack`).
pub fn cluster_poses(state: &ItemRenderState, pose: Mat4, amount: u32, seed: i64) -> Vec<Mat4> {
    if amount == 0 {
        return Vec::new();
    }
    let (lo, hi) = state.bounding_box();
    cluster_poses_with_depth(pose, amount, seed, hi.z - lo.z)
}

/// [`cluster_poses`] with the depth of the resolved model's bounding box
/// already computed for this draw (also used to place the stack's bob).
pub fn cluster_poses_with_depth(pose: Mat4, amount: u32, seed: i64, depth: f32) -> Vec<Mat4> {
    use horizon_core::random::{LegacyRandom, RandomSource};
    let mut out = Vec::new();
    if amount == 0 {
        return out;
    }
    let mut random = LegacyRandom::new(seed);
    let mut next = || random.next_float() * 2.0 - 1.0;
    if depth > 0.0625 {
        out.push(pose);
        for _ in 1..amount {
            let (x, y, z) = (next() * 0.15, next() * 0.15, next() * 0.15);
            out.push(pose * Mat4::from_translation(Vec3::new(x, y, z)));
        }
    } else {
        let offset = depth * 1.5;
        let mut p = pose
            * Mat4::from_translation(Vec3::new(0.0, 0.0, -(offset * (amount - 1) as f32 / 2.0)));
        out.push(p);
        p *= Mat4::from_translation(Vec3::new(0.0, 0.0, offset));
        for _ in 1..amount {
            let (x, y) = (next() * 0.15 * 0.5, next() * 0.15 * 0.5);
            out.push(p * Mat4::from_translation(Vec3::new(x, y, 0.0)));
            p *= Mat4::from_translation(Vec3::new(0.0, 0.0, offset));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_world::native_load_context::DataLoadOrigin;
    use std::fs;

    /// Item definitions are read again on every load: a node of a registered type keeps its
    /// `models` as the packs now give them, and a definition with a type no mod registered isn't
    /// loaded, so its item draws the missing model.
    #[test]
    fn a_registered_item_model_type_is_read_again_on_each_load() {
        use horizon_assets::resource_source::Resource;
        horizon_assets::item_model::register_item_models(vec!["loadtest:cell".into()]);
        let source = |full: &str| {
            let cell = format!(
                r#"{{"model":{{"type":"loadtest:cell","models":{{"full":{{"type":"minecraft:model","model":"{full}"}}}}}}}}"#
            );
            ResourceSource::indexed(BTreeMap::from([
                (
                    "assets/loadtest/items/cell.json".to_owned(),
                    Resource::bytes(cell.into_bytes()),
                ),
                (
                    "assets/loadtest/items/other.json".to_owned(),
                    Resource::bytes(br#"{"model":{"type":"loadtest:unregistered"}}"#.to_vec()),
                ),
            ]))
        };
        let models = |full: &str| {
            let loaded = load_definitions(&source(full));
            assert!(!loaded.contains_key("loadtest:other"));
            let mut models = Vec::new();
            loaded["loadtest:cell"].model.dependencies(&mut models);
            models
        };
        assert_eq!(models("loadtest:item/a"), ["loadtest:item/a"]);
        assert_eq!(models("loadtest:item/b"), ["loadtest:item/b"]);
    }

    /// A pack's own item models, nested in conditions too, are baked with their corners before
    /// anything draws them, as vanilla bakes every model when it loads the pack; the `minecraft`
    /// namespace's aren't.
    #[test]
    fn a_packs_own_item_models_are_baked_ahead() {
        let root = std::env::temp_dir().join(format!("horizon-bake-ahead-{}", std::process::id()));
        let write = |name: &str, data: &str| {
            let path = root.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, data).unwrap();
        };
        write("items.json", "{}");
        write(
            "assets/minecraft/atlases/items.json",
            r#"{"sources":[{"type":"single","resource":"track:item/rail"}]}"#,
        );
        super::glint_tests::png(
            &root.join("assets/track/textures/item/rail.png"),
            16,
            16,
            [90, 90, 90, 255],
        );
        write(
            "assets/track/items/rail.json",
            r#"{"model":{"type":"minecraft:model","model":"track:item/rail"}}"#,
        );
        write(
            "assets/track/models/item/rail.json",
            r##"{"textures":{"all":"track:item/rail"},"elements":[{"from":[0,0,0],"to":[16,2,16],
                "faces":{"up":{"uv":[0,0,16,16],"texture":"#all"}}}]}"##,
        );
        write(
            "assets/track/items/signal.json",
            r#"{"model":{"type":"minecraft:condition","property":"minecraft:using_item",
                "on_true":{"type":"minecraft:model","model":"track:item/signal_on"},
                "on_false":{"type":"minecraft:model","model":"track:item/rail"}}}"#,
        );
        write(
            "assets/track/models/item/signal_on.json",
            r##"{"textures":{"all":"track:item/rail"},"elements":[{"from":[7,0,7],"to":[9,16,9],
                "faces":{"north":{"uv":[0,0,2,16],"texture":"#all"},
                "south":{"uv":[0,0,2,16],"texture":"#all"}}}]}"##,
        );
        write(
            "assets/minecraft/items/stick.json",
            r#"{"model":{"type":"minecraft:model","model":"minecraft:item/stick"}}"#,
        );
        let data = crate::test_resources::item_data(&root, DataLoadOrigin::ClientPreload).unwrap();
        let renderer =
            Arc::new(ItemRenderer::load(&ResourceRoot::directory(root.clone()), &data).unwrap());
        ItemRenderer::bake_ahead(&Arc::downgrade(&renderer), renderer.pack_model_names());
        let baked = renderer.baked.lock().unwrap();
        assert_eq!(baked["track:item/rail"].quads.len(), 1);
        assert_eq!(baked["track:item/signal_on"].quads.len(), 2);
        assert!(!baked.contains_key("minecraft:item/stick"));
        assert_eq!(renderer.extents.lock().unwrap()["track:item/rail"].len(), 4);
        drop(baked);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    #[ignore = "daily"]
    fn archive_item_models_use_real_prepared_tables_without_unpacking() -> crate::Result<()> {
        use horizon_assets::resource_source::Resource;
        use std::io::Write;
        struct Scratch(std::path::PathBuf);
        impl Drop for Scratch {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let prepared = std::path::PathBuf::from(
            std::env::var_os("HORIZON_TEST_RESOURCES").ok_or("resources required")?,
        );
        let scratch = Scratch(std::env::temp_dir().join(format!(
                "horizon-real-item-source-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_nanos()
            )));
        fs::create_dir(&scratch.0)?;
        let mut image = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut image, 1, 1);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()?
                .write_image_data(&[20, 30, 40, 255])?;
        }
        let jar = scratch.0.join("items.jar");
        let mut writer = zip::ZipWriter::new(fs::File::create(&jar)?);
        for (name, bytes) in [
            ("assets/archive_item_smoke/models/item/panel.json", br#"{"parent":"minecraft:item/generated","textures":{"layer0":"archive_item_smoke:item/panel"}}"#.as_slice()),
            ("assets/archive_item_smoke/items/panel.json", br#"{"model":{"type":"minecraft:model","model":"archive_item_smoke:item/panel"}}"#.as_slice()),
            ("assets/archive_item_smoke/textures/item/panel.png", image.as_slice()),
            ("entity-models.json", b"invalid archive table".as_slice()),
            ("items.json", b"invalid archive table".as_slice()),
        ] {
            writer.start_file(name, zip::write::SimpleFileOptions::default())?;
            writer.write_all(bytes)?;
        }
        writer.finish()?;
        let base = ResourceSource::directory(prepared.clone());
        let mut files: BTreeMap<_, _> = base
            .list("assets/")
            .into_iter()
            .map(|name| (name.clone(), Resource::file(base.clone(), name)))
            .collect();
        let archive = ResourceSource::archive(&jar)?;
        for name in archive.list("") {
            files.insert(name.clone(), Resource::file(archive.clone(), name));
        }
        let root = ResourceRoot::new(
            prepared.clone(),
            scratch.0.join("cache"),
            ResourceSource::indexed(files),
        );
        let data = crate::test_resources::item_data(
            &prepared,
            horizon_world::native_load_context::DataLoadOrigin::ClientPreload,
        )?;
        let renderer = ItemRenderer::load(&root, &data)?;
        assert!(
            renderer
                .models
                .definitions
                .contains_key("archive_item_smoke:panel")
        );
        assert!(
            !renderer.parts.is_empty(),
            "prepared entity models stay separate from archive entries"
        );
        let model = renderer.bake("archive_item_smoke:item/panel");
        assert!(!model.quads.is_empty());
        assert_eq!(model.particle.sprite, "archive_item_smoke:item/panel");
        assert!(!prepared.join("assets/archive_item_smoke").exists());
        assert!(!scratch.0.join("assets").exists());
        Ok(())
    }

    #[test]
    fn archive_item_definitions_masks_and_glint_need_no_extracted_files() {
        use std::io::Write;
        let mut png = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png, 1, 2);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&[1, 2, 3, 0, 4, 5, 6, 128])
                .unwrap();
        }
        let folder = std::env::temp_dir().join(format!(
            "horizon-item-source-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&folder).unwrap();
        let jar = folder.join("items.jar");
        let mut writer = zip::ZipWriter::new(fs::File::create(&jar).unwrap());
        for (name, bytes) in [
            ("assets/example/textures/item/mask.png", png.as_slice()),
            (
                "assets/example/textures/item/mask.png.mcmeta",
                br#"{"animation":{"width":1,"height":1,"frames":[0,1]}}"#.as_slice(),
            ),
            (
                "assets/example/items/nested/mask.json",
                br#"{"model":{"type":"minecraft:empty"}}"#.as_slice(),
            ),
            (
                "assets/minecraft/atlases/items.json",
                br#"{"sources":[{"type":"minecraft:single","resource":"example:item/mask"}]}"#
                    .as_slice(),
            ),
        ] {
            writer
                .start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap();
        let source = ResourceSource::archive(&jar).unwrap();
        fs::remove_file(&jar).unwrap();
        assert!(load_definitions(&source).contains_key("example:nested/mask"));
        let sprites = FileSprites {
            root: source.clone(),
            permutations: Vec::new(),
            cache: Mutex::default(),
            translucent: Mutex::default(),
        };
        let mask = sprites.mask("example:item/mask").unwrap();
        assert_eq!((mask.width, mask.height), (1, 1));
        assert_eq!(mask.frames, vec![vec![true], vec![false]]);
        assert!(sprites.translucent("example:item/mask"));
        let atlas = glint_atlas::load(&source, 1, 1024).unwrap();
        assert!(atlas.contains_key("example:item/mask"));
        assert_eq!(fs::read_dir(&folder).unwrap().count(), 0);
        fs::remove_dir(&folder).unwrap();
    }

    #[test]
    fn custom_model_flags_select_their_model_after_stack_nbt_decoding() {
        let mut actual = Vec::new();
        for flags in [
            Tag::List(vec![Tag::Byte(1), Tag::Byte(0)]),
            Tag::ByteArray(vec![1, 0]),
        ] {
            let mut stack = crate::items::ItemStack::new("minecraft:diamond", 1);
            let component = Tag::compound([
                ("flags", flags),
                ("floats", Tag::List(vec![Tag::Float(1.25f32.to_bits())])),
                ("colors", Tag::List(vec![Tag::Int(0x12_34_56)])),
                ("strings", Tag::List(vec![Tag::String("keep".into())])),
            ]);
            (stack.damage, stack.extra) = crate::item_data::decode_components(Some(
                &Tag::compound([("minecraft:custom_model_data", component)]),
            ));
            let view = stack_view(&stack);
            for index in 0..3 {
                let mut models = ItemModels::default();
                Arc::make_mut(&mut models.prototypes).insert(
                    "minecraft:diamond".into(),
                    BTreeMap::from([(
                        "minecraft:item_model".into(),
                        serde_json::json!("minecraft:diamond"),
                    )]),
                );
                models.definitions.insert("minecraft:diamond".into(), ClientItem::decode(&serde_json::json!({
                    "model": {"type": "minecraft:condition", "property": "minecraft:custom_model_data", "index": index,
                        "on_true": {"type": "minecraft:model", "model": "horizon:item/yes"},
                        "on_false": {"type": "minecraft:model", "model": "horizon:item/no"}}
                })).unwrap());
                let resolved = models.resolve(
                    &view,
                    &Context {
                        display: DisplayContext::Gui,
                        owner: None,
                        level: None,
                        seed: 0,
                    },
                    &horizon_assets::item_model::NoEnv,
                );
                assert_eq!(resolved.layers.len(), 1);
                actual.push(
                    resolved.layers[0].content
                        == LayerContent::Model {
                            model: "horizon:item/yes".into(),
                        },
                );
            }
            // Non-boolean fields must retain their codec's numeric/string values.
            let component = view.patch["minecraft:custom_model_data"].as_ref().unwrap();
            assert_eq!(component["floats"], serde_json::json!([1.25]));
            assert_eq!(component["colors"], serde_json::json!([0x12_34_56]));
            assert_eq!(component["strings"], serde_json::json!(["keep"]));
        }
        assert_eq!(actual, [true, false, false, true, false, false]);
    }

    #[test]
    fn custom_name_and_written_book_nbt_match_component_select_models() {
        let bread_def = ClientItem::decode(&serde_json::json!({
            "model": {
                "type": "minecraft:select",
                "property": "minecraft:component",
                "component": "minecraft:custom_name",
                "cases": [{
                    "when": {
                        "text": "",
                        "extra": [{
                            "text": "Styled Item",
                            "strikethrough": false,
                            "obfuscated": false,
                            "bold": false,
                            "italic": true,
                            "underlined": false,
                            "color": "gold"
                        }]
                    },
                    "model": {"type": "minecraft:model", "model": "example:item/styled"}
                }],
                "fallback": {"type": "minecraft:model", "model": "minecraft:item/bread"}
            }
        }))
        .unwrap();
        let book_def = ClientItem::decode(&serde_json::json!({
            "model": {
                "type": "minecraft:select",
                "property": "minecraft:component",
                "component": "minecraft:written_book_content",
                "cases": [{
                    "when": {
                        "generation": 1,
                        "pages": [{"raw": "Page 1\nLine 2"}],
                        "resolved": true,
                        "title": {"raw": "Ledger"},
                        "author": "Alice"
                    },
                    "model": {"type": "minecraft:model", "model": "example:item/ledger"}
                }],
                "fallback": {"type": "minecraft:model", "model": "minecraft:item/written_book"}
            }
        }))
        .unwrap();
        let egg_def = ClientItem::decode(&serde_json::json!({
            "model": {
                "type": "select",
                "property": "component",
                "component": "entity_data",
                "cases": [{
                    "when": {"id": "frog", "variant": "cold"},
                    "model": {"type": "minecraft:model", "model": "example:item/cold_variant"}
                }],
                "fallback": {"type": "minecraft:model", "model": "minecraft:item/frog_spawn_egg"}
            }
        }))
        .unwrap();

        let mut models = ItemModels::default();
        models
            .definitions
            .insert("minecraft:bread".into(), bread_def);
        models
            .definitions
            .insert("minecraft:written_book".into(), book_def);
        models
            .definitions
            .insert("minecraft:frog_spawn_egg".into(), egg_def);
        for id in [
            "minecraft:bread",
            "minecraft:written_book",
            "minecraft:frog_spawn_egg",
        ] {
            Arc::make_mut(&mut models.prototypes).insert(
                id.into(),
                BTreeMap::from([("minecraft:item_model".into(), Value::String(id.into()))]),
            );
        }
        let ctx = Context {
            display: DisplayContext::Gui,
            owner: None,
            level: None,
            seed: 0,
        };
        let resolve = |stack: &crate::items::ItemStack| {
            let resolved =
                models.resolve(&stack_view(stack), &ctx, &horizon_assets::item_model::NoEnv);
            let LayerContent::Model { model } = &resolved.layers[0].content else {
                panic!("expected model");
            };
            model.clone()
        };

        let (name_nbt, _) = horizon_core::nbt::parse_snbt(
            r#"{text:"",extra:[{text:"Styled Item",strikethrough:0b,obfuscated:0b,bold:0b,italic:1b,underlined:0b,color:"gold"}]}"#,
        )
        .unwrap();
        let mut bread = crate::items::ItemStack::new("minecraft:bread", 1);
        (bread.damage, bread.extra) =
            crate::item_data::decode_components(Some(&Tag::compound([(
                "minecraft:custom_name",
                name_nbt,
            )])));
        assert_eq!(resolve(&bread), "example:item/styled");

        let book_nbt = Tag::compound([
            ("title", Tag::String("Ledger".into())),
            ("author", Tag::String("Alice".into())),
            ("generation", Tag::Int(1)),
            (
                "pages",
                Tag::List(vec![Tag::String("Page 1\nLine 2".into())]),
            ),
            ("resolved", Tag::Byte(1)),
        ]);
        let mut book = crate::items::ItemStack::new("minecraft:written_book", 1);
        (book.damage, book.extra) = crate::item_data::decode_components(Some(&Tag::compound([(
            "minecraft:written_book_content",
            book_nbt,
        )])));
        assert_eq!(resolve(&book), "example:item/ledger");

        let egg_nbt = Tag::compound([
            ("id", Tag::String("minecraft:frog".into())),
            ("variant", Tag::String("cold".into())),
        ]);
        let mut egg = crate::items::ItemStack::new("minecraft:frog_spawn_egg", 1);
        (egg.damage, egg.extra) = crate::item_data::decode_components(Some(&Tag::compound([(
            "minecraft:entity_data",
            egg_nbt,
        )])));
        assert_eq!(resolve(&egg), "example:item/cold_variant");
    }

    #[test]
    fn posed_item_bounds_include_each_cluster_transform() {
        let state = ItemRenderState {
            display: DisplayContext::Ground,
            layers: vec![LayerRender {
                geometry: LayerGeometry::Special {
                    model: SpecialModel {
                        kind: horizon_assets::item_model::SpecialKind::Trident,
                    },
                    argument: Value::Null,
                },
                tints: Vec::new(),
                foil: FoilType::None,
                matrix: Mat4::from_translation(Vec3::new(0.0, 2.0, 0.0)),
                particle: String::new(),
                uses_block_light: false,
                extents: Arc::new(vec![
                    Vec3::new(-1.0, -0.5, -0.25),
                    Vec3::new(0.5, 1.0, 0.25),
                ]),
            }],
            oversized_in_gui: false,
            animated: false,
            cit_glint: None,
            hdr_tier: 0,
            hdr_light: 0,
        };
        let poses = [
            Mat4::from_translation(Vec3::new(2.0, 0.0, 0.0)),
            Mat4::from_translation(Vec3::new(-1.0, 0.0, 0.0)),
        ];
        let (lo, hi) = state.posed_bounds(&poses).expect("item model has vertices");
        assert_eq!(lo, Vec3::new(-2.0, 1.5, -0.25));
        assert_eq!(hi, Vec3::new(2.5, 3.0, 0.25));
    }

    #[test]
    fn stack_copy_positions_keep_the_flat_and_solid_random_sequences() {
        // Captured from the original cluster renderer with seed 7, three
        // copies and an identity pose. Flat items draw two random values
        // per extra copy; solid items draw three.
        for (depth, expected) in [
            (
                0.03125,
                [
                    [0, 0, 3_175_088_128],
                    [1_024_310_737, 1_017_789_485, 0],
                    [1_025_054_461, 3_180_766_298, 1_027_604_480],
                ],
            ),
            (
                0.25,
                [
                    [0, 0, 0],
                    [1_032_699_345, 1_026_178_093, 1_033_443_069],
                    [3_189_154_906, 3_174_720_933, 3_141_056_154],
                ],
            ),
        ] {
            let copies = cluster_poses_with_depth(Mat4::IDENTITY, 3, 7, depth);
            assert_eq!(copies.len(), expected.len());
            for (pose, position) in copies.iter().zip(expected) {
                let expected =
                    Mat4::from_translation(Vec3::from_array(position.map(f32::from_bits)));
                assert_eq!(
                    pose.to_cols_array().map(f32::to_bits),
                    expected.to_cols_array().map(f32::to_bits)
                );
            }
        }
        assert!(cluster_poses_with_depth(Mat4::IDENTITY, 0, 7, 0.25).is_empty());
    }

    /// One frame's glint is read once and shared: every item drawn from the
    /// same `GlintFrame` scrolls by the same phase, and without the atlas
    /// sprite no item gets a glint (`TextureTransform.setupGlintTexturing`
    /// once per batch, `GameRenderer.extractOptions` once per frame).
    #[test]
    fn a_frame_glint_is_shared_by_every_item() {
        let frame = GlintFrame {
            millis: 123_456.0,
            speed: 0.5,
            alpha: 0.75,
            sprite: true,
        };
        let world = frame.glint(0.5).expect("the atlas has the glint sprite");
        assert_eq!(
            (world.speed, world.alpha, world.millis),
            (0.5, 0.75, 123_456.0)
        );
        assert_eq!(world.scale, 0.5);
        let uvs: Vec<_> = (0..1000)
            .map(|_| frame.glint(0.5).unwrap().uv([0.25, 0.75]))
            .collect();
        assert!(uvs.iter().all(|uv| *uv == uvs[0]));
        assert_eq!(frame.scroll_ticks(), Glint::scroll_ticks(123_456.0, 0.5));
        // 123456 ms * 0.5 * 8 = 493824 ticks, modulo 330000.
        assert_eq!(frame.scroll_ticks(), 163_824.0);
        let missing = GlintFrame {
            sprite: false,
            ..frame
        };
        assert!(missing.glint(0.5).is_none());
        assert_eq!(missing.scroll_ticks(), frame.scroll_ticks());
    }

    /// A bad item model surfaces only once items.json has loaded, so a broken
    /// items.json still reports itself first.
    #[test]
    fn item_model_errors_wait_for_the_item_definitions() -> crate::Result<()> {
        let root =
            std::env::temp_dir().join(format!("horizon-item-model-errors-{}", std::process::id()));
        let model = root.join("assets/horizon/models/INVALID.json");
        fs::create_dir_all(model.parent().unwrap())?;
        fs::write(model, "{}")?;
        let library = crate::model_library::ParsedModels::load(
            &crate::resource_source::ResourceSource::directory(root.clone()),
        )
        .into_item_library();
        assert!(library.is_err());
        fs::write(root.join("items.json"), "{")?;
        let load = || {
            crate::test_resources::item_data(&root, DataLoadOrigin::ClientPreload)
                .and_then(|data| {
                    ItemRenderer::load_with_library(
                        &horizon_assets::resource_source::ResourceRoot::directory(
                            root.to_path_buf(),
                        ),
                        library.clone(),
                        &data,
                    )
                })
                .err()
                .unwrap()
                .to_string()
        };
        let broken_items = load();
        fs::write(root.join("items.json"), r#"{"horizon:bad":false}"#)?;
        let data = crate::test_resources::item_data(&root, DataLoadOrigin::ClientPreload)?;
        let broken_components = ItemRenderer::load_with_library(
            &horizon_assets::resource_source::ResourceRoot::directory(root.to_path_buf()),
            library.clone(),
            &data,
        )
        .err()
        .unwrap()
        .to_string();
        fs::write(root.join("items.json"), "{}")?;
        let bad_model = load();
        let _ = fs::remove_dir_all(&root);
        assert!(!broken_items.starts_with("item models:"));
        assert!(!broken_components.starts_with("item models:"));
        assert!(bad_model.starts_with("item models:"));
        Ok(())
    }

    /// Definitions live at any depth (`items/<dir>/<name>.json`), as
    /// ModelEngine's `modelengine:postman/head` does.
    #[test]
    fn item_definitions_load_from_nested_folders() -> crate::Result<()> {
        let root = std::env::temp_dir().join(format!(
            "horizon-item-definitions-nested-{}",
            std::process::id()
        ));
        let definitions = root.join("assets/horizon/items");
        fs::create_dir_all(definitions.join("postman"))?;
        let empty = r#"{"model":{"type":"minecraft:empty"}}"#;
        fs::write(definitions.join("top.json"), empty)?;
        fs::write(definitions.join("postman/head.json"), empty)?;
        let loaded = load_definitions(&ResourceSource::directory(root.clone()));
        let _ = fs::remove_dir_all(&root);
        assert!(loaded.contains_key("horizon:top"));
        assert!(loaded.contains_key("horizon:postman/head"));
        Ok(())
    }

    /// A dedicated server keeps the stacks' default components but reads
    /// none of the client item definitions.
    #[test]
    fn a_server_keeps_prototypes_without_definitions() -> crate::Result<()> {
        let root = std::env::temp_dir().join(format!("horizon-item-server-{}", std::process::id()));
        let definitions = root.join("assets/horizon/items");
        fs::create_dir_all(&definitions)?;
        fs::write(
            definitions.join("top.json"),
            r#"{"model":{"type":"minecraft:empty"}}"#,
        )?;
        fs::write(
            root.join("items.json"),
            r#"{"horizon:top":{"minecraft:max_stack_size":16}}"#,
        )?;
        let cit = root.join("assets/minecraft/optifine");
        fs::create_dir_all(&cit)?;
        fs::write(cit.join("cit.properties"), "useGlint=false\n")?;
        let library = crate::model_library::ParsedModels::load(
            &crate::resource_source::ResourceSource::directory(root.clone()),
        )
        .into_item_library();
        let data = crate::test_resources::item_data(&root, DataLoadOrigin::ClientPreload)?;
        let client = ItemRenderer::load_with_library(
            &horizon_assets::resource_source::ResourceRoot::directory(root.to_path_buf()),
            library.clone(),
            &data,
        )?;
        let server = ItemRenderer::load_for_server(
            &horizon_assets::resource_source::ResourceRoot::directory(root.to_path_buf()),
            library,
            &data,
        )?;
        let _ = fs::remove_dir_all(&root);
        assert!(client.models.definitions.contains_key("horizon:top"));
        assert!(server.models.definitions.is_empty());
        assert!(client.cit.is_some());
        assert!(server.cit.is_none());
        assert_eq!(server.models.prototypes, client.models.prototypes);
        Ok(())
    }

    #[test]
    fn rendered_amounts_follow_the_count_bands() {
        let bands: Vec<u32> = [0, 1, 2, 16, 17, 32, 33, 48, 49, 64]
            .iter()
            .map(|c| rendered_amount(*c))
            .collect();
        assert_eq!(bands, [1, 1, 2, 2, 3, 3, 4, 4, 5, 5]);
    }
    #[test]
    fn nbt_components_read_as_json() {
        let tag = Tag::compound([
            ("a", Tag::Int(3)),
            ("b", Tag::List(vec![Tag::String("x".into())])),
            ("c", Tag::Float(1.5f32.to_bits())),
            ("d", Tag::Byte(1)),
        ]);
        assert_eq!(
            tag_json(&tag),
            serde_json::json!({"a": 3, "b": ["x"], "c": 1.5, "d": 1})
        );
    }
    #[test]
    fn stack_glint_override_reaches_the_resolved_item_layers() {
        use horizon_assets::item_model::NoEnv;
        let mut models = ItemModels::default();
        models.definitions.insert(
            "horizon:fixture".into(),
            ClientItem::from_json(
                r#"{"model":{"type":"minecraft:model","model":"horizon:item/fixture"}}"#,
            )
            .unwrap(),
        );
        for default in [false, true] {
            Arc::make_mut(&mut models.prototypes).insert(
                "minecraft:diamond".into(),
                BTreeMap::from([
                    (
                        "minecraft:item_model".into(),
                        serde_json::json!("horizon:fixture"),
                    ),
                    (
                        "minecraft:enchantment_glint_override".into(),
                        Value::Bool(default),
                    ),
                ]),
            );
            for (tag, flag) in [
                (Tag::Byte(1), true),
                (Tag::Byte(0), false),
                (Tag::Byte(-1), true),
                (Tag::Short(256), true),
                (Tag::Int(1), true),
                (Tag::Long(1 << 40), true),
                (Tag::Float(0.5_f32.to_bits()), true),
                (Tag::Float((-0.0_f32).to_bits()), false),
                (Tag::Double(0.5_f64.to_bits()), true),
                (Tag::Double((-0.0_f64).to_bits()), false),
                (Tag::Float(f32::NAN.to_bits()), true),
                (Tag::Double(f64::INFINITY.to_bits()), true),
            ] {
                let mut stack = crate::items::ItemStack::new("minecraft:diamond", 1);
                (stack.damage, stack.extra) = crate::item_data::decode_components(Some(
                    &Tag::compound([("minecraft:enchantment_glint_override", tag)]),
                ));
                assert_eq!(stack.has_foil(Some(default)), flag);
                let view = stack_view(&stack);
                assert_eq!(
                    view.patch["minecraft:enchantment_glint_override"],
                    Some(Value::Bool(flag))
                );
                let resolved = models.resolve(
                    &view,
                    &Context {
                        display: DisplayContext::Gui,
                        owner: None,
                        level: None,
                        seed: 0,
                    },
                    &NoEnv,
                );
                assert_eq!(resolved.layers.len(), 1);
                assert_eq!(
                    resolved.layers[0].foil,
                    if flag {
                        FoilType::Standard
                    } else {
                        FoilType::None
                    }
                );
            }
        }
    }

    #[test]
    fn palette_trns_partial_alpha_counts_as_translucent() {
        // Slime/honey (and ice) are indexed PNGs with a tRNS chunk. Decode must
        // keep that fractional alpha so FileSprites marks them translucent (#4508).
        let png: &[u8] = &[
            0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x02, 0x08, 0x03, 0x00, 0x00,
            0x00, 0x45, 0x68, 0xfd, 0x16, 0x00, 0x00, 0x00, 0x06, 0x50, 0x4c, 0x54, 0x45, 0x00,
            0xff, 0x00, 0x00, 0x80, 0x00, 0x54, 0x80, 0xe6, 0x08, 0x00, 0x00, 0x00, 0x01, 0x74,
            0x52, 0x4e, 0x53, 0xb4, 0x8c, 0xea, 0xaf, 0xf3, 0x00, 0x00, 0x00, 0x0e, 0x49, 0x44,
            0x41, 0x54, 0x78, 0x9c, 0x63, 0x60, 0x60, 0x64, 0x60, 0x60, 0x04, 0x00, 0x00, 0x0b,
            0x00, 0x03, 0x52, 0x29, 0x78, 0x8f, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44,
            0xae, 0x42, 0x60, 0x82,
        ];
        let dir = std::env::temp_dir().join("horizon-trns-alpha");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("indexed.png");
        std::fs::write(&path, png).unwrap();
        let (_w, _h, rgba) = crate::hud::decode(&path).unwrap();
        let partial = rgba.chunks(4).filter(|p| matches!(p[3], 1..=254)).count();
        assert!(partial > 0, "tRNS alpha was stripped: {rgba:?}");
    }

    #[test]
    fn normals_follow_the_inverse_transpose() {
        let m = Mat4::from_scale(Vec3::new(2.0, 1.0, 1.0));
        assert_eq!(transform_normal(m, Vec3::X), Vec3::X);
        let r = Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2);
        assert!((transform_normal(r, Vec3::Z) - Vec3::X).length() < 1e-6);
    }

    /// Every vanilla item on the ground, held in either hand, framed and worn on the
    /// head, turned, shaded and unshaded: the GPU path's vertices (its jobs expanded as
    /// `entity_skin.wgsl` does) are the CPU path's, to the bit and in the same order, for
    /// each state it takes. Needs the prepared resources.
    #[test]
    fn items_posed_on_the_gpu_draw_what_the_cpu_draws() {
        use crate::gfx::entity_skin::{SkinFrame, expand};
        use crate::mobs::skin::{Skin, SkinModels};
        use horizon_assets::item_model::{NoEnv, StackView};
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("entity-models.json").exists() {
            eprintln!("no prepared resources: skipped");
            return;
        }
        let data = crate::test_resources::item_data(&root, DataLoadOrigin::ClientPreload).unwrap();
        let r = ItemRenderer::load(
            &horizon_assets::resource_source::ResourceRoot::directory(root.to_path_buf()),
            &data,
        )
        .unwrap();
        let resolve = |id: &str| {
            let view = StackView::new(id, 1);
            r.resolve(&view, DisplayContext::Ground, None, None, 7, &NoEnv)
        };
        // Each display context's transforms (`ItemTransform`s) pose the same quads.
        let contexts = [
            DisplayContext::Ground,
            DisplayContext::ThirdPersonRightHand,
            DisplayContext::ThirdPersonLeftHand,
            DisplayContext::Fixed,
            DisplayContext::Head,
        ];
        let states: Vec<_> = r
            .models
            .definitions
            .keys()
            .flat_map(|id| {
                contexts.map(|context| {
                    let view = StackView::new(id, 1);
                    (id, r.resolve(&view, context, None, None, 7, &NoEnv))
                })
            })
            .collect();
        // Each sprite its own rect, so a run drawn with another's shows.
        let mut sprites: HashMap<String, Rect> = HashMap::new();
        for (_, state) in &states {
            for layer in &state.layers {
                if let LayerGeometry::Quads(baked) = &layer.geometry {
                    for q in &baked.quads {
                        let n = sprites.len() as f32;
                        sprites
                            .entry(sprite_name(&q.material.sprite).to_owned())
                            .or_insert(Rect {
                                u0: n / 4096.0,
                                v0: 0.5 - n / 8192.0,
                                u1: n / 4096.0 + 0.003,
                                v1: 0.5 - n / 8192.0 + 0.007,
                                w: 16,
                                h: 16,
                            });
                    }
                }
            }
        }
        let pose = Mat4::from_translation(Vec3::new(-2.3, 0.7, 5.1))
            * Mat4::from_rotation_y(2.1)
            * Mat4::from_scale(Vec3::splat(0.5));
        let mut models = SkinModels::default();
        let mut frame = SkinFrame::default();
        let (mut taken, mut left) = (0, 0);
        for lights in [
            Some(crate::block_entity_render::diffuse_lights(false)),
            None,
        ] {
            for (id, state) in &states {
                let mut cpu = Vec::new();
                let mut sink = WorldSink {
                    out: &mut cpu,
                    light: 0x50_a0,
                    lights,
                    glint: None,
                    tier: 1,
                };
                state.submit(&r, &sprites, &NoHost, pose, &mut sink);
                frame.clear_draws();
                let mut skin = Skin::new(&mut models, &mut frame);
                if !state.submit_skinned(&r, &sprites, pose, (0x50_a0, lights, 1), &mut skin) {
                    assert!(frame.jobs.is_empty(), "{id}: left to the CPU after drawing");
                    left += 1;
                    continue;
                }
                taken += 1;
                let bounds = skin.bounds;
                let gpu = expand(&frame.new_model, &frame);
                assert_eq!(gpu.len(), cpu.len(), "{id}");
                for (i, (g, c)) in gpu.iter().zip(&cpu).enumerate() {
                    assert_eq!(
                        bytemuck::bytes_of(g),
                        bytemuck::bytes_of(c),
                        "{id} vertex {i}: {g:?} != {c:?}"
                    );
                }
                if let (Some((lo, hi)), false) = (bounds, cpu.is_empty()) {
                    for v in &cpu {
                        let p = Vec3::from_array(v.pos);
                        assert!(
                            p.cmpge(lo - 1e-4).all() && p.cmple(hi + 1e-4).all(),
                            "{id}: {p} outside"
                        );
                    }
                }
            }
        }
        // The 8 kinds of the `stress_items` scene all take it, and resolved again (as
        // every frame does) they draw the models already stored.
        let stored = frame.new_model.len();
        for id in [
            "stone",
            "oak_log",
            "diamond",
            "apple",
            "iron_sword",
            "bread",
            "redstone",
            "torch",
        ] {
            frame.clear_draws();
            let mut skin = Skin::new(&mut models, &mut frame);
            let state = resolve(&format!("minecraft:{id}"));
            assert!(
                state.submit_skinned(&r, &sprites, pose, (0, None, 0), &mut skin),
                "{id}"
            );
        }
        assert_eq!(frame.new_model.len(), stored, "models stored again");
        eprintln!("{taken} item states on the GPU path, {left} on the CPU");
        assert!(taken > left, "{taken} taken, {left} left");
    }

    #[test]
    fn face_normals_of_a_pose_are_its_inverse_transpose() {
        // A dropped item's pose: spun, squashed and moved. Each face's normal is the
        // inverse transpose of the pose's 3x3 applied to the face's axis, renormalized,
        // here worked out in f64 from the cofactors.
        let m = Mat4::from_translation(Vec3::new(3.0, -1.0, 7.5))
            * Mat4::from_rotation_y(0.7)
            * Mat4::from_rotation_x(-0.3)
            * Mat4::from_scale(Vec3::new(0.5, 0.25, 2.0));
        let c = m.to_cols_array();
        let a = |row: usize, col: usize| f64::from(c[col * 4 + row]);
        let cofactor = |row: usize, col: usize| {
            let (r0, r1) = ((row + 1) % 3, (row + 2) % 3);
            let (c0, c1) = ((col + 1) % 3, (col + 2) % 3);
            a(r0, c0) * a(r1, c1) - a(r0, c1) * a(r1, c0)
        };
        let mut normals = FaceNormals::new(m);
        // Twice each: a cached face gives what it gave the first time.
        for d in Direction::ALL.into_iter().chain(Direction::ALL) {
            let axis = direction_vector(d).to_array().map(f64::from);
            // inverse^T = cofactors / det: the det's sign is the only part that survives
            // renormalizing, and it is positive here.
            let n: Vec<f64> = (0..3)
                .map(|row| (0..3).map(|col| cofactor(row, col) * axis[col]).sum())
                .collect();
            let len = n.iter().map(|v| v * v).sum::<f64>().sqrt();
            let got = normals.face(d);
            for (g, e) in got.to_array().into_iter().zip(&n) {
                assert!(
                    (f64::from(g) - e / len).abs() < 1e-6,
                    "{d:?}: {got} vs {n:?}"
                );
            }
            assert_eq!(got, transform_normal(m, direction_vector(d)), "{d:?}");
        }
    }
}
