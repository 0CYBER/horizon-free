//! The geometry a mod's model loader answers (`bake_model_loader`).
use crate::Direction;

/// One quad a model loader adds to a model whose `loader` it registered
/// (`register_model_loaders`), baked with the model's elements: turned with the
/// blockstate variant, culled and shaded as element faces are.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: mods construct quads and the game reads every field
#[derive(horizon_mod_api_macros::Wire)]
pub struct ModelLoaderQuad {
    /// Corners in block units (0 to 1 across the block), counter-clockwise seen from the front,
    /// as an obj model's are.
    pub positions: [[f32; 3]; 4],
    /// Each corner's texture coordinates, 0 to 1 across the sprite.
    pub uvs: [[f32; 2]; 4],
    /// The texture: a key of the model's `textures` (`side`, or `#side`).
    pub texture: String,
    /// The face whose solid neighbour hides the quad; `None` never culls.
    pub cull: Option<Direction>,
    /// The tint index a block tint source colours it by; -1 is none.
    pub tint_index: i32,
    /// Whether it takes directional shading.
    pub shade: bool,
}
