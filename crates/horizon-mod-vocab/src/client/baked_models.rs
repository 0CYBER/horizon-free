//! The client-native model catalog borrowed by `modify_baking_result`.
use crate::Direction;

/// A model in the current block and standalone bake, addressed without renderer slots.
// no wire: bake-local lookup key with a borrowed name, used only by the native catalog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum BakedModelKey<'a> {
    /// A transformed model shared by the block states that select it.
    Variant {
        /// The resource identifier, such as `minecraft:block/oak_stairs`.
        model: &'a str,
        /// X, Y, Z blockstate rotations in quarter turns.
        rotations: [u8; 3],
        /// Whether the bake locked the UVs to the unrotated world faces.
        uv_lock: bool,
    },
    /// A registered standalone identifier, including the result of a derived form.
    Standalone(&'a str),
}

/// One already transformed quad, borrowed directly from the current bake.
// no wire: borrows native baked geometry and sprite metadata for the callback.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct BakedModelQuad<'a> {
    /// Four positions in model-local block units, in draw order.
    pub positions: &'a [[f32; 3]; 4],
    /// UVs in the quad's sprite, in the same order as `positions`; 1 is one sprite width or height.
    pub uvs: &'a [[f32; 2]; 4],
    /// The sprite resource identifier, not the atlas layer (which can be shared).
    pub sprite: &'a str,
    /// Facing derived from the baked vertex order.
    pub facing: Direction,
    /// The neighbour face that can hide this quad, if any.
    pub cull_face: Option<Direction>,
    /// Tint channel, or `None` if the quad does not request a tint.
    pub tint_index: Option<i32>,
    /// Whether directional shading is enabled.
    pub shade: bool,
    /// Baked directional-shading override, if the source supplies one.
    pub shade_direction_override: Option<Direction>,
    /// Per-quad ambient occlusion override; `None` inherits the model's flag.
    pub ambient_occlusion: Option<bool>,
    /// Explicit ARGB colour multiplied into the tint, if the source supplies one.
    pub color: Option<i32>,
    /// The minimum `[block, sky]` light the quad is drawn with.
    pub light: [u8; 2],
}

/// An allocation-free view of the candidate model's geometry.
// no wire: borrows a live native model through its face access trait.
#[derive(Clone, Copy)]
#[non_exhaustive]
pub struct BakedModelView<'a> {
    faces: &'a dyn BakedModelFaces,
}

impl<'a> BakedModelView<'a> {
    /// The default ambient occlusion flag for the model's quads.
    pub fn ambient_occlusion(self) -> bool {
        self.faces.ambient_occlusion()
    }

    /// Visit the quads in baked order, without cloning their geometry or names.
    pub fn quads(self) -> impl ExactSizeIterator<Item = BakedModelQuad<'a>> + 'a {
        (0..self.faces.quad_count()).map(move |index| self.faces.quad(index))
    }
}

/// A bake-time edit of one quad, keeping its material, tint and lighting metadata on replacement.
// no wire: edit returned to the native bake callback, not a Wasm operation.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub enum BakedQuadChange {
    /// Keep this quad exactly as baked.
    Keep,
    /// Omit this quad from the published model.
    Remove,
    /// Replace the geometry in place, with the original sprite and draw attributes.
    Replace {
        /// Four positions in model-local block units. Their winding determines the new facing.
        positions: [[f32; 3]; 4],
        /// The new sprite-relative UVs.
        uvs: [[f32; 2]; 4],
        /// Which neighbour face can hide this quad. Use `None` for uncullable geometry.
        cull_face: Option<Direction>,
    },
}

/// Why a requested bake-time edit could not be applied.
// no wire: error from the native catalog editor, which has no Wasm entry point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum BakedModelEditError {
    /// The named model is not in this bake.
    UnknownModel,
    /// A replacement has non-finite coordinates, UVs outside `[0, 2)`, or no usable facing. No edits to the model were applied.
    InvalidGeometry,
}

/// The live candidate for this resource bake, only available during `modify_baking_result`.
// no wire: exclusive borrow of the native resource bake's in-progress catalog.
#[non_exhaustive]
pub struct BakedModels<'a> {
    catalog: &'a mut dyn BakedModelsAccess,
}

impl BakedModels<'_> {
    /// Borrow a model by its actual resource identifier and transform or by registered standalone id.
    pub fn model(&self, key: BakedModelKey<'_>) -> Option<BakedModelView<'_>> {
        self.catalog.model(key)
    }

    /// Visit every possible baked variant of this exact block state, without sampling its render RNG.
    /// Use a namespaced block id and all its properties, sorted by property name (or `&[]` for no
    /// properties). Visits all weighted alternatives and only matching multipart parts, in order;
    /// the same model may occur more than once. Returns false if the state has no model binding.
    pub fn visit_block_state<'a>(
        &'a self,
        block: &str,
        properties: &[(&str, &str)],
        mut visit: impl FnMut(BakedModelKey<'a>, BakedModelView<'a>),
    ) -> bool {
        self.catalog
            .visit_block_state(block, properties, &mut visit)
    }

    /// Inspect and edit one baked model's quads in order. Returning `Keep` for all quads copies
    /// no geometry and allocates nothing. An invalid replacement leaves the whole model untouched.
    /// The edit applies to all uses of this shared bake (including a standalone that shares it).
    /// The editor runs now, not when a model renders, and is not kept by the game.
    pub fn edit_quads(
        &mut self,
        key: BakedModelKey<'_>,
        mut edit: impl FnMut(BakedModelQuad<'_>) -> BakedQuadChange,
    ) -> Result<(), BakedModelEditError> {
        self.catalog.edit_quads(key, &mut edit)
    }
}

#[allow(missing_docs)]
pub mod built {
    use super::*;

    pub trait BakedModelFaces {
        fn ambient_occlusion(&self) -> bool;
        fn quad_count(&self) -> usize;
        fn quad(&self, index: usize) -> BakedModelQuad<'_>;
    }

    pub trait BakedModelsAccess {
        fn model(&self, key: BakedModelKey<'_>) -> Option<BakedModelView<'_>>;
        fn visit_block_state<'a>(
            &'a self,
            block: &str,
            properties: &[(&str, &str)],
            visit: &mut dyn FnMut(BakedModelKey<'a>, BakedModelView<'a>),
        ) -> bool;
        fn edit_quads(
            &mut self,
            key: BakedModelKey<'_>,
            edit: &mut dyn FnMut(BakedModelQuad<'_>) -> BakedQuadChange,
        ) -> Result<(), BakedModelEditError>;
    }

    #[inline]
    pub fn baked_model_quad<'a>(
        positions: &'a [[f32; 3]; 4],
        uvs: &'a [[f32; 2]; 4],
        sprite: &'a str,
        facing: Direction,
        cull_face: Option<Direction>,
        tint_index: Option<i32>,
        shade: bool,
        shade_direction_override: Option<Direction>,
        ambient_occlusion: Option<bool>,
        color: Option<i32>,
        light: [u8; 2],
    ) -> BakedModelQuad<'a> {
        BakedModelQuad {
            positions,
            uvs,
            sprite,
            facing,
            cull_face,
            tint_index,
            shade,
            shade_direction_override,
            ambient_occlusion,
            color,
            light,
        }
    }

    impl<'a> BakedModelView<'a> {
        pub fn new(faces: &'a dyn BakedModelFaces) -> Self {
            Self { faces }
        }
    }

    impl<'a> BakedModels<'a> {
        pub fn new(catalog: &'a mut dyn BakedModelsAccess) -> Self {
            Self { catalog }
        }
    }
}

use built::{BakedModelFaces, BakedModelsAccess};
