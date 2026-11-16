//! Baking of item models: a cuboid model's quads, or the layer extrusion of
//! `builtin/generated` (`ItemModelGenerator`), with the display transforms
//! and render properties a `CuboidItemModelWrapper` keeps.
use crate::{
    bake::{ModelQuad, bake_block_model, bake_flat_face},
    item_model::DisplayContext,
    json::Result,
    math::{Mat4, quaternion_xyz},
    model::{Direction, DisplayTransform, GuiLight, ItemTransform, Material, TextureSlot},
    resolve::{GENERATED_MODEL, ModelLibrary, ResolvedCuboidModel},
};
use std::{collections::BTreeMap, collections::BTreeSet, f32::consts::PI};

pub const MISSING_SPRITE: &str = "minecraft:missingno";

/// The alpha of every unique animation frame of a sprite.
#[derive(Clone, Debug, PartialEq)]
pub struct SpriteMask {
    pub width: usize,
    pub height: usize,
    /// `frames[f][y * width + x]` is true for alpha 0.
    pub frames: Vec<Vec<bool>>,
}
impl SpriteMask {
    pub fn opaque(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            frames: vec![vec![false; width * height]],
        }
    }
    fn transparent(&self, frame: usize, x: i64, y: i64) -> bool {
        if x < 0 || y < 0 || x >= self.width as i64 || y >= self.height as i64 {
            return true;
        }
        self.frames[frame][y as usize * self.width + x as usize]
    }
    /// This mask's per-frame transparency, a pixel opaque in any frame opaque in the result
    /// (`SpriteContents.getUniqueFrames`, which the mask quads union).
    fn union(&self) -> Vec<bool> {
        let mut opaque = vec![false; self.width * self.height];
        for frame in &self.frames {
            for (o, transparent) in opaque.iter_mut().zip(frame) {
                *o |= !*transparent;
            }
        }
        opaque
    }
}
/// The sprites of the item and block atlases.
pub trait SpriteLookup {
    /// `None` when the sprite is in neither atlas (the missing sprite stands in).
    fn mask(&self, sprite: &str) -> Option<SpriteMask>;
}

#[derive(Clone, Debug)]
pub struct BakedItemModel {
    /// `QuadCollection.getAll`: unculled quads, then north, south, east,
    /// west, up and down.
    pub quads: Vec<ModelQuad>,
    /// `gui_light: side`.
    pub uses_block_light: bool,
    pub particle: Material,
    /// By [`crate::model::DISPLAY_CONTEXTS`]; `None` is `NO_TRANSFORM`.
    pub transforms: [Option<ItemTransform>; 9],
}

fn context_index(context: DisplayContext) -> Option<usize> {
    Some(match context {
        DisplayContext::None => return None,
        DisplayContext::ThirdPersonLeftHand => 0,
        DisplayContext::ThirdPersonRightHand => 1,
        DisplayContext::FirstPersonLeftHand => 2,
        DisplayContext::FirstPersonRightHand => 3,
        DisplayContext::Head => 4,
        DisplayContext::Gui => 5,
        DisplayContext::Ground => 6,
        DisplayContext::Fixed => 7,
        DisplayContext::OnShelf => 8,
    })
}
impl BakedItemModel {
    /// `ItemTransforms.getTransform`.
    pub fn transform(&self, context: DisplayContext) -> Option<&ItemTransform> {
        context_index(context).and_then(|i| self.transforms[i].as_ref())
    }
    /// `LayerRenderState.applyTransform`: the display transform followed by
    /// the item model's own local transformation.
    pub fn layer_matrix(&self, context: DisplayContext, local: &Mat4) -> Mat4 {
        item_transform_matrix(self.transform(context), context.left_hand()).mul(local)
    }
}
/// `ItemTransform.apply` on an identity pose.
pub fn item_transform_matrix(transform: Option<&ItemTransform>, left_hand: bool) -> Mat4 {
    let Some(t) = transform else {
        return Mat4::IDENTITY.translate([-0.5, -0.5, -0.5]);
    };
    let (tx, ry, rz) = if left_hand {
        (-t.translation[0], -t.rotation[1], -t.rotation[2])
    } else {
        (t.translation[0], t.rotation[1], t.rotation[2])
    };
    let radians = PI / 180.0;
    Mat4::IDENTITY
        .translate([tx, t.translation[1], t.translation[2]])
        .rotate(quaternion_xyz(
            t.rotation[0] * radians,
            ry * radians,
            rz * radians,
        ))
        .scale(t.scale)
        .translate([-0.5, -0.5, -0.5])
}

/// Bake the top geometry of `name` the way `CuboidItemModelWrapper` does.
pub fn bake_item_model(
    library: &ModelLibrary,
    sprites: &dyn SpriteLookup,
    name: &str,
) -> Result<BakedItemModel> {
    let model = library
        .resolve(name)
        .or_else(|_| library.resolve(crate::resolve::MISSING_MODEL))?;
    bake_resolved(&model, sprites)
}

/// The flat item model a layer sprite hangs off (`ItemModelGenerator`'s parent).
const ITEM_GENERATED_MODEL: &str = "minecraft:item/generated";

/// One `item/generated` layer of `sprite`, its shape the opaque pixels of `mask` when
/// set: `ItemModelGenerator.bakeExtrudedSprite` for a base layer, NeoForge's
/// `UnbakedElementsHelper.bakeItemMaskQuads` for a fluid or cover layer.
pub fn bake_sprite_layer(
    library: &ModelLibrary,
    sprites: &dyn SpriteLookup,
    sprite: &str,
    mask: Option<&str>,
) -> Result<BakedItemModel> {
    let base = if library.resolves(ITEM_GENERATED_MODEL) {
        ITEM_GENERATED_MODEL
    } else {
        GENERATED_MODEL
    };
    let textures = BTreeMap::from([(
        "layer0".to_owned(),
        TextureSlot::Material(Material {
            sprite: sprite.to_owned(),
            force_translucent: false,
        }),
    )]);
    let model = library.resolve_with_textures(base, &textures)?;
    bake_layers(&model, sprites, mask)
}

/// Bake an already resolved model, e.g. one from
/// [`ModelLibrary::resolve_with_textures`].
pub fn bake_resolved(
    model: &ResolvedCuboidModel,
    sprites: &dyn SpriteLookup,
) -> Result<BakedItemModel> {
    bake_layers(model, sprites, None)
}

/// `mask` gives the first layer its shape (NeoForge's mask quads): its own sprite is
/// drawn on the front and back, the mask sprite's opaque pixels decide where, and the
/// side faces show the mask sprite itself.
fn bake_layers(
    model: &ResolvedCuboidModel,
    sprites: &dyn SpriteLookup,
    mask: Option<&str>,
) -> Result<BakedItemModel> {
    let known = |sprite: &str| sprite == MISSING_SPRITE || sprites.mask(sprite).is_some();
    let material = |m: &Material| {
        if known(&m.sprite) {
            m.clone()
        } else {
            Material {
                sprite: MISSING_SPRITE.to_owned(),
                force_translucent: m.force_translucent,
            }
        }
    };
    let quads = if model.uses_item_generator() {
        let mut quads = Vec::new();
        for (layer, slot) in ["layer0", "layer1", "layer2", "layer3", "layer4"]
            .into_iter()
            .enumerate()
        {
            let Some(m) = model.material(slot)? else {
                break;
            };
            let m = material(m);
            if layer == 0
                && let Some(mask) = mask
            {
                // A mask in neither atlas is the missing sprite (`MaterialBaker.get`), which is
                // opaque: the layer fills the whole square, its sides `missingno`.
                let shape = sprites
                    .mask(mask)
                    .unwrap_or_else(|| SpriteMask::opaque(16, 16));
                let mask_material = material(&Material {
                    sprite: mask.to_owned(),
                    force_translucent: false,
                });
                quads.extend(extrude_masked(&shape, &m, &mask_material, layer as i32)?);
                continue;
            }
            let layer_mask = if m.sprite == MISSING_SPRITE {
                SpriteMask::opaque(16, 16)
            } else {
                sprites
                    .mask(&m.sprite)
                    .unwrap_or_else(|| SpriteMask::opaque(16, 16))
            };
            quads.extend(extrude_layer(&layer_mask, &m, layer as i32)?);
        }
        quads
    } else {
        let mut quads =
            bake_block_model(model, crate::bake::BlockModelTransform::new([0; 3], false))?;
        for q in &mut quads {
            q.material = material(&q.material);
        }
        // `QuadCollection.Builder.build`: unculled, then each culled direction.
        let mut ordered: Vec<ModelQuad> =
            quads.iter().filter(|q| q.cull.is_none()).cloned().collect();
        for side in [
            Direction::North,
            Direction::South,
            Direction::East,
            Direction::West,
            Direction::Up,
            Direction::Down,
        ] {
            ordered.extend(quads.iter().filter(|q| q.cull == Some(side)).cloned());
        }
        ordered
    };
    // `resolveParticleMaterial`: a missing `particle` slot is the missing sprite.
    let particle = model.material("particle")?.map_or_else(
        || Material {
            sprite: MISSING_SPRITE.to_owned(),
            force_translucent: false,
        },
        material,
    );
    let transforms = std::array::from_fn(|i| match &model.display.contexts[i] {
        DisplayTransform::Explicit(Some(t)) => Some(t.clone()),
        _ => None,
    });
    Ok(BakedItemModel {
        quads,
        uses_block_light: model.gui_light == GuiLight::Side,
        particle,
        transforms,
    })
}
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Side {
    Up,
    Down,
    Left,
    Right,
}
impl Side {
    fn step(self) -> (i64, i64) {
        match self {
            Self::Up => (0, 1),
            Self::Down => (0, -1),
            Self::Left => (1, 0),
            Self::Right => (-1, 0),
        }
    }
    fn direction(self) -> Direction {
        match self {
            Self::Up => Direction::Up,
            Self::Down => Direction::Down,
            Self::Left => Direction::East,
            Self::Right => Direction::West,
        }
    }
}

/// `ItemModelGenerator.bakeExtrudedSprite` for one layer: the front and back
/// faces and one thin face per opaque/transparent pixel edge. The native
/// client collects the sides in a hash set, so their order is unspecified;
/// they are emitted sorted by pixel here.
fn extrude_layer(mask: &SpriteMask, material: &Material, layer: i32) -> Result<Vec<ModelQuad>> {
    let from = [0.0, 0.0, 7.5];
    let to = [16.0, 16.0, 8.5];
    let mut quads = vec![
        bake_flat_face(
            from,
            to,
            [0.0, 0.0, 16.0, 16.0],
            Direction::South,
            material.clone(),
            layer,
        )?,
        bake_flat_face(
            from,
            to,
            [16.0, 0.0, 0.0, 16.0],
            Direction::North,
            material.clone(),
            layer,
        )?,
    ];
    quads.extend(side_faces(mask, material, layer)?);
    Ok(quads)
}

/// `UnbakedElementsHelper.bakeItemMaskQuads`: one thin face per edge of `shape`'s
/// opaque pixels (which `ItemModelGenerator.bakeSideFaces` emits first), then the
/// front and back faces over merged runs of those pixels, not over the whole sprite.
/// The sides are baked from the mask's own material with tint index -1 (the 5-argument
/// overload's `MaterialInfo.of(mask, ..., -1, ...)`, offsets 17-50); only the front and
/// back draw `material` with the layer's tint index.
fn extrude_masked(
    shape: &SpriteMask,
    material: &Material,
    mask_material: &Material,
    layer: i32,
) -> Result<Vec<ModelQuad>> {
    let mut quads = side_faces(shape, mask_material, -1)?;
    quads.extend(mask_rect_faces(shape, material, layer)?);
    Ok(quads)
}

/// One thin face per opaque/transparent edge, over every animation frame.
fn side_faces(mask: &SpriteMask, material: &Material, layer: i32) -> Result<Vec<ModelQuad>> {
    let (width, height) = (mask.width as i64, mask.height as i64);
    let mut faces = BTreeSet::new();
    for frame in 0..mask.frames.len() {
        for y in 0..height {
            for x in 0..width {
                if mask.transparent(frame, x, y) {
                    continue;
                }
                for side in [Side::Up, Side::Down, Side::Left, Side::Right] {
                    let (sx, sy) = side.step();
                    if mask.transparent(frame, x - sx, y - sy) {
                        faces.insert((y, x, side));
                    }
                }
            }
        }
    }
    let x_scale = 16.0 / mask.width as f32;
    let y_scale = 16.0 / mask.height as f32;
    let mut quads = Vec::new();
    for (y, x, side) in faces {
        let (x, y) = (x as f32, y as f32);
        let u0 = x + 0.1;
        let u1 = x + 1.0 - 0.1;
        let (v0, v1) = if matches!(side, Side::Up | Side::Down) {
            (y + 0.1, y + 1.0 - 0.1)
        } else {
            (y + 1.0 - 0.1, y + 0.1)
        };
        let (mut start_x, mut start_y, mut end_x, mut end_y) = (x, y, x, y);
        match side {
            Side::Up => end_x += 1.0,
            Side::Down => {
                end_x += 1.0;
                start_y += 1.0;
                end_y += 1.0;
            }
            Side::Left => end_y += 1.0,
            Side::Right => {
                start_x += 1.0;
                end_x += 1.0;
                end_y += 1.0;
            }
        }
        start_x *= x_scale;
        end_x *= x_scale;
        start_y *= y_scale;
        end_y *= y_scale;
        start_y = 16.0 - start_y;
        end_y = 16.0 - end_y;
        let (from, to) = match side {
            Side::Up => ([start_x, start_y, 7.5], [end_x, start_y, 8.5]),
            Side::Down => ([start_x, end_y, 7.5], [end_x, end_y, 8.5]),
            Side::Left => ([start_x, start_y, 7.5], [start_x, end_y, 8.5]),
            Side::Right => ([end_x, start_y, 7.5], [end_x, end_y, 8.5]),
        };
        quads.push(bake_flat_face(
            from,
            to,
            [u0 * x_scale, v0 * y_scale, u1 * x_scale, v1 * y_scale],
            side.direction(),
            material.clone(),
            layer,
        )?);
    }
    Ok(quads)
}

/// The front and back faces of a masked layer. The rule: the layer's sprite is drawn on
/// both faces over exactly the mask's opaque pixels (opaque in any of its frames), every
/// one of them and nothing else, so the mask alone decides where the layer is visible.
/// The pixels are merged into rectangles as NeoForge's `bakeItemMaskQuads` does: each
/// row's run of still-unused opaque pixels, grown down while every one of the run's own
/// columns stays opaque, becomes one South and one North quad with UVs over the same
/// sub-rectangle.
///
/// Two deliberate departures from NeoForge 26.3's loop, which would leave holes: a run
/// that reaches the sprite's last column is drawn (NeoForge closes a run only on a
/// transparent pixel, so such a run loses its front and back), and growing down tests
/// only the run's own columns (NeoForge also tests the transparent pixel that closed the
/// run). Both were agreed on review of arismoko/horizon#2707.
fn mask_rect_faces(shape: &SpriteMask, material: &Material, layer: i32) -> Result<Vec<ModelQuad>> {
    let (width, height) = (shape.width, shape.height);
    let mut opaque = shape.union();
    let mut quads = Vec::new();
    for y in 0..height {
        let mut x = 0;
        while x < width {
            if !opaque[y * width + x] {
                x += 1;
                continue;
            }
            let start = x;
            while x < width && opaque[y * width + x] {
                x += 1;
            }
            let mut bottom = y + 1;
            while bottom < height && (start..x).all(|cx| opaque[bottom * width + cx]) {
                bottom += 1;
            }
            for cy in y..bottom {
                for cx in start..x {
                    opaque[cy * width + cx] = false;
                }
            }
            for direction in [Direction::South, Direction::North] {
                quads.push(mask_rect_face(
                    shape,
                    material,
                    layer,
                    (start, x, y, bottom),
                    direction,
                )?);
            }
        }
    }
    Ok(quads)
}

/// One front or back face over the sprite's pixels `x0..x1` by `y0..y1`, its UVs the
/// same sub-rectangle (`FaceBakery.defaultFaceUV`).
fn mask_rect_face(
    shape: &SpriteMask,
    material: &Material,
    layer: i32,
    (x0, x1, y0, y1): (usize, usize, usize, usize),
    direction: Direction,
) -> Result<ModelQuad> {
    let (width, height) = (shape.width as f32, shape.height as f32);
    let (u0, u1) = (16.0 * x0 as f32 / width, 16.0 * x1 as f32 / width);
    let (v0, v1) = (16.0 * y0 as f32 / height, 16.0 * y1 as f32 / height);
    let from = [u0, 16.0 - v1, 7.5];
    let to = [u1, 16.0 - v0, 8.5];
    let uv = match direction {
        Direction::South => [u0, v0, u1, v1],
        _ => [u1, v0, u0, v1],
    };
    bake_flat_face(from, to, uv, direction, material.clone(), layer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CuboidModel, TextureSlot};
    use std::collections::BTreeMap;

    /// The stick: 2x2 with one transparent pixel. `item/cit`: one opaque pixel, so a bake
    /// shows whose mask it extruded. A fluid sprite and its masks: one pixel, an L, a
    /// fully opaque square and a fully transparent one.
    struct Sprites;
    impl SpriteLookup for Sprites {
        fn mask(&self, sprite: &str) -> Option<SpriteMask> {
            match sprite {
                "minecraft:item/stick" => Some(SpriteMask {
                    width: 2,
                    height: 2,
                    frames: vec![vec![true, false, false, false]],
                }),
                "horizon_cit:item/cit" => Some(SpriteMask {
                    width: 1,
                    height: 1,
                    frames: vec![vec![false]],
                }),
                "mod:item/fluid" => Some(SpriteMask {
                    width: 2,
                    height: 2,
                    frames: vec![vec![false, false, false, false]],
                }),
                // One opaque pixel: the bottom-right.
                "mod:item/mask" => Some(SpriteMask {
                    width: 2,
                    height: 2,
                    frames: vec![vec![true, true, true, false]],
                }),
                // An L of three opaque pixels, the bottom-right one transparent.
                "mod:item/l" => Some(SpriteMask {
                    width: 2,
                    height: 2,
                    frames: vec![vec![false, false, false, true]],
                }),
                // Two frames, each with one opaque pixel: the top-left in the first, the
                // centre in the second. Neither touches the last column.
                "mod:item/blink" => Some(SpriteMask {
                    width: 3,
                    height: 3,
                    frames: vec![
                        vec![false, true, true, true, true, true, true, true, true],
                        vec![true, true, true, true, false, true, true, true, true],
                    ],
                }),
                "mod:item/clear" => Some(SpriteMask {
                    width: 2,
                    height: 2,
                    frames: vec![vec![true, true, true, true]],
                }),
                _ => None,
            }
        }
    }
    fn library() -> ModelLibrary {
        ModelLibrary::with_item_generator(BTreeMap::from(
            [
                ("item/generated", r#"{"parent":"builtin/generated","display":{"gui":{"scale":[2,2,2]}}}"#),
                ("item/stick", r#"{"parent":"item/generated","textures":{"layer0":"item/stick"}}"#),
                ("horizon:loop", r#"{"parent":"horizon:loop2"}"#),
                ("horizon:loop2", r#"{"parent":"horizon:loop"}"#),
                ("block/slab", r##"{"textures":{"particle":"#side","side":"block/absent"},"elements":[{"from":[0,0,0],"to":[16,8,16],"faces":{"up":{"texture":"#side"},"down":{"texture":"#side","cullface":"down"}}}]}"##),
            ]
            .map(|(name, json)| (name.to_owned(), CuboidModel::from_json(json).unwrap())),
        ))
        .unwrap()
    }
    #[test]
    fn baking_by_name_falls_back_to_the_missing_model() {
        let library = library();
        // Front, back, and the 8 unit edges around the L of 3 opaque pixels.
        let stick = bake_item_model(&library, &Sprites, "item/stick").unwrap();
        assert_eq!(stick.quads.len(), 10);
        // An unknown sprite bakes as the missing sprite.
        let slab = bake_item_model(&library, &Sprites, "block/slab").unwrap();
        assert_eq!(slab.quads.len(), 2);
        assert_eq!(slab.particle.sprite, MISSING_SPRITE);
        // A cyclic chain, an error to `resolve`, bakes as the missing cube.
        let cyclic = bake_item_model(&library, &Sprites, "horizon:loop").unwrap();
        assert_eq!(cyclic.quads.len(), 6);
        assert!(
            cyclic
                .quads
                .iter()
                .all(|q| q.material.sprite == MISSING_SPRITE)
        );
    }
    #[test]
    fn replaced_layer_bakes_with_its_own_sprite() {
        let library = library();
        let child = BTreeMap::from([(
            "layer0".to_owned(),
            TextureSlot::Material(Material {
                sprite: "horizon_cit:item/cit".to_owned(),
                force_translucent: false,
            }),
        )]);
        let model = library.resolve_with_textures("item/stick", &child).unwrap();
        let baked = bake_resolved(&model, &Sprites).unwrap();
        // The CIT sprite's own mask, one opaque pixel: front, back and 4 edges.
        assert_eq!(baked.quads.len(), 6);
        assert!(
            baked
                .quads
                .iter()
                .all(|q| q.material.sprite == "horizon_cit:item/cit")
        );
        assert_eq!(baked.particle.sprite, "horizon_cit:item/cit");
        assert_eq!(baked.transforms[5].as_ref().unwrap().scale, [2.0; 3]);
    }
    /// A quad's model-space bounds (its `from`..`to` after the /16 bake).
    fn extent(quad: &ModelQuad) -> ([f32; 3], [f32; 3]) {
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        for corner in &quad.positions {
            for (i, value) in corner.iter().enumerate() {
                min[i] = min[i].min(*value);
                max[i] = max[i].max(*value);
            }
        }
        (min, max)
    }
    /// A quad's texture coordinates in 0..16 units, sorted so corner order is not pinned.
    fn uv_corners(quad: &ModelQuad) -> Vec<[i32; 2]> {
        let mut corners: Vec<[i32; 2]> = quad
            .uv
            .iter()
            .map(|uv| [(uv[0] * 16.0).round() as i32, (uv[1] * 16.0).round() as i32])
            .collect();
        corners.sort();
        corners
    }
    fn assert_face(
        quad: &ModelQuad,
        direction: Direction,
        from: [f32; 3],
        to: [f32; 3],
        uv: [[i32; 2]; 4],
    ) {
        assert_eq!(quad.facing, direction);
        let (min, max) = extent(quad);
        for i in 0..3 {
            assert!((min[i] - from[i]).abs() < 1e-6, "{min:?} != {from:?}");
            assert!((max[i] - to[i]).abs() < 1e-6, "{max:?} != {to:?}");
        }
        let mut expected = uv.to_vec();
        expected.sort();
        assert_eq!(uv_corners(quad), expected);
    }

    #[test]
    fn a_sprite_layer_is_extruded_through_its_mask() {
        let library = library();
        // The fluid sprite is four opaque pixels: front, back and 8 edges.
        let fluid = bake_sprite_layer(&library, &Sprites, "mod:item/fluid", None).unwrap();
        assert_eq!(fluid.quads.len(), 10);
        assert_eq!(fluid.particle.sprite, "mod:item/fluid");
        // The mask keeps the bottom-right pixel: its 4 edges, and front and back over
        // that pixel only (NeoForge's mask quads), both drawn with the fluid sprite.
        let masked =
            bake_sprite_layer(&library, &Sprites, "mod:item/fluid", Some("mod:item/mask")).unwrap();
        assert_eq!(masked.quads.len(), 6);
        let faces: Vec<&ModelQuad> = masked
            .quads
            .iter()
            .filter(|q| matches!(q.facing, Direction::South | Direction::North))
            .collect();
        assert_eq!(faces.len(), 2);
        assert_face(
            faces[0],
            Direction::South,
            [0.5, 0.0, 8.5 / 16.0],
            [1.0, 0.5, 8.5 / 16.0],
            [[8, 8], [8, 16], [16, 16], [16, 8]],
        );
        assert_face(
            faces[1],
            Direction::North,
            [0.5, 0.0, 7.5 / 16.0],
            [1.0, 0.5, 7.5 / 16.0],
            [[8, 8], [8, 16], [16, 16], [16, 8]],
        );
        // The mask is the shape; the sprite is what the front and back draw, with the
        // layer's tint index 0. The 4 sides show the mask sprite itself, untinted (-1).
        for face in &faces {
            assert_eq!(face.material.sprite, "mod:item/fluid");
            assert_eq!(face.tint_index, 0);
        }
        let sides: Vec<&ModelQuad> = masked
            .quads
            .iter()
            .filter(|q| !matches!(q.facing, Direction::South | Direction::North))
            .collect();
        assert_eq!(sides.len(), 4);
        for side in &sides {
            assert_eq!(side.material.sprite, "mod:item/mask");
            assert_eq!(side.tint_index, -1);
        }
        // A sprite layer takes `item/generated`'s displays, as NeoForge reads its
        // top transforms.
        assert_eq!(masked.transforms[5].as_ref().unwrap().scale, [2.0; 3]);
    }

    /// A mask no atlas holds is `missingno` (`MaterialBaker.get`), opaque over 16x16: the
    /// drawn sprite fills the whole front and back, and the 64 one-pixel sides show
    /// `missingno`, untinted.
    #[test]
    fn a_missing_mask_is_the_opaque_missing_sprite() {
        let library = library();
        let masked = bake_sprite_layer(
            &library,
            &Sprites,
            "mod:item/fluid",
            Some("mod:item/absent"),
        )
        .unwrap();
        assert_eq!(masked.quads.len(), 64 + 2);
        let (faces, sides): (Vec<&ModelQuad>, Vec<&ModelQuad>) = masked
            .quads
            .iter()
            .partition(|q| matches!(q.facing, Direction::South | Direction::North));
        assert_eq!(faces.len(), 2);
        assert_face(
            faces[0],
            Direction::South,
            [0.0, 0.0, 8.5 / 16.0],
            [1.0, 1.0, 8.5 / 16.0],
            [[0, 0], [0, 16], [16, 16], [16, 0]],
        );
        assert!(faces.iter().all(|q| q.material.sprite == "mod:item/fluid"));
        assert!(
            sides
                .iter()
                .all(|q| q.material.sprite == MISSING_SPRITE && q.tint_index == -1)
        );
    }

    /// An animated mask's shape is every pixel opaque in any of its frames: NeoForge's
    /// `bakeItemMaskQuads` sets a pixel's bit for each of `getUniqueFrames` (offsets 91-109),
    /// and `ItemModelGenerator.bakeSideFaces` collects each frame's edges. Two frames with
    /// disjoint opaque pixels, the top-left and the centre of a 3x3 mask, give each pixel its
    /// own 4 sides and its own front and back: 8 + 4 quads.
    #[test]
    fn an_animated_mask_draws_the_union_of_its_frames() {
        let library = library();
        let masked =
            bake_sprite_layer(&library, &Sprites, "mod:item/fluid", Some("mod:item/blink"))
                .unwrap();
        assert_eq!(masked.quads.len(), 8 + 4);
        let faces: Vec<&ModelQuad> = masked
            .quads
            .iter()
            .filter(|q| matches!(q.facing, Direction::South | Direction::North))
            .collect();
        assert_eq!(faces.len(), 4);
        // Top-left pixel: x 0..1/3, y 2/3..1; texture 0..16/3 on both axes.
        assert_face(
            faces[0],
            Direction::South,
            [0.0, 2.0 / 3.0, 8.5 / 16.0],
            [1.0 / 3.0, 1.0, 8.5 / 16.0],
            [[0, 0], [0, 5], [5, 5], [5, 0]],
        );
        assert_face(
            faces[1],
            Direction::North,
            [0.0, 2.0 / 3.0, 7.5 / 16.0],
            [1.0 / 3.0, 1.0, 7.5 / 16.0],
            [[0, 0], [0, 5], [5, 5], [5, 0]],
        );
        // Centre pixel: x 1/3..2/3, y 1/3..2/3; texture 16/3..32/3.
        assert_face(
            faces[2],
            Direction::South,
            [1.0 / 3.0, 1.0 / 3.0, 8.5 / 16.0],
            [2.0 / 3.0, 2.0 / 3.0, 8.5 / 16.0],
            [[5, 5], [5, 11], [11, 11], [11, 5]],
        );
        assert_face(
            faces[3],
            Direction::North,
            [1.0 / 3.0, 1.0 / 3.0, 7.5 / 16.0],
            [2.0 / 3.0, 2.0 / 3.0, 7.5 / 16.0],
            [[5, 5], [5, 11], [11, 11], [11, 5]],
        );
        let sides = masked
            .quads
            .iter()
            .filter(|q| !matches!(q.facing, Direction::South | Direction::North));
        assert!(
            sides
                .into_iter()
                .all(|q| q.material.sprite == "mod:item/blink" && q.tint_index == -1)
        );
    }

    #[test]
    fn a_transparent_mask_bakes_no_faces() {
        let library = library();
        let masked =
            bake_sprite_layer(&library, &Sprites, "mod:item/fluid", Some("mod:item/clear"))
                .unwrap();
        assert!(masked.quads.is_empty());
    }

    /// Horizon's rule, not NeoForge's loop: the top row reaches the last column and is
    /// still drawn (see `mask_rect_faces`).
    #[test]
    fn a_partial_mask_merges_its_runs_into_one_face_pair_each() {
        let library = library();
        let masked =
            bake_sprite_layer(&library, &Sprites, "mod:item/fluid", Some("mod:item/l")).unwrap();
        // Three opaque pixels, all on the sprite's border: 2 + 3 + 3 edges, then the
        // greedy merge's two rectangles (top row's two pixels, then the lower-left one),
        // each a front and a back face.
        assert_eq!(masked.quads.len(), 8 + 4);
        let faces: Vec<&ModelQuad> = masked
            .quads
            .iter()
            .filter(|q| matches!(q.facing, Direction::South | Direction::North))
            .collect();
        assert_eq!(faces.len(), 4);
        // The top row's both columns: x 0..16, y 8..16 of a 16-high sprite.
        assert_face(
            faces[0],
            Direction::South,
            [0.0, 0.5, 8.5 / 16.0],
            [1.0, 1.0, 8.5 / 16.0],
            [[0, 0], [0, 8], [16, 8], [16, 0]],
        );
        assert_face(
            faces[1],
            Direction::North,
            [0.0, 0.5, 7.5 / 16.0],
            [1.0, 1.0, 7.5 / 16.0],
            [[0, 0], [0, 8], [16, 8], [16, 0]],
        );
        // The lower-left pixel: x 0..8, y 0..8.
        assert_face(
            faces[2],
            Direction::South,
            [0.0, 0.0, 8.5 / 16.0],
            [0.5, 0.5, 8.5 / 16.0],
            [[0, 8], [0, 16], [8, 16], [8, 8]],
        );
        assert_face(
            faces[3],
            Direction::North,
            [0.0, 0.0, 7.5 / 16.0],
            [0.5, 0.5, 7.5 / 16.0],
            [[0, 8], [0, 16], [8, 16], [8, 8]],
        );
    }

    /// Horizon's rule, not NeoForge's loop: a fully opaque mask keeps its front and back
    /// (NeoForge's loop draws none, every run reaching the last column).
    #[test]
    fn a_full_mask_keeps_one_front_and_back() {
        let library = library();
        // The fluid sprite's own four opaque pixels mask the same way it draws.
        let masked =
            bake_sprite_layer(&library, &Sprites, "mod:item/fluid", Some("mod:item/fluid"))
                .unwrap();
        assert_eq!(masked.quads.len(), 10);
        let faces: Vec<&ModelQuad> = masked
            .quads
            .iter()
            .filter(|q| matches!(q.facing, Direction::South | Direction::North))
            .collect();
        assert_eq!(faces.len(), 2);
        assert_face(
            faces[0],
            Direction::South,
            [0.0, 0.0, 8.5 / 16.0],
            [1.0, 1.0, 8.5 / 16.0],
            [[0, 0], [0, 16], [16, 16], [16, 0]],
        );
        assert_face(
            faces[1],
            Direction::North,
            [0.0, 0.0, 7.5 / 16.0],
            [1.0, 1.0, 7.5 / 16.0],
            [[0, 0], [0, 16], [16, 16], [16, 0]],
        );
    }
}
