//! Standalone models drawn with the terrain's own materials by block entity, mob and player
//! submit hooks ([`BlockEntityDrawContext::terrain_model`](crate::BlockEntityDrawContext::terrain_model)).

/// Which terrain material a [`TerrainModelOptions`] draw uses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum TerrainMaterial {
    /// Each quad's own: solid, `cutout` or `cutout_mipped` as its model's `render_type` names
    /// it. A translucent quad, or a render type the game doesn't know, refuses the whole draw.
    #[default]
    Model,
    /// Opaque, with no alpha test.
    Solid,
    /// Texels under alpha 0.1 discarded, sampled without mipmaps.
    Cutout,
    /// Texels under alpha 0.5 discarded, sampled with mipmaps.
    CutoutMipped,
}

/// How a [`TerrainModelOptions`] draw shades faces by their direction. Light never changes it:
/// a full-bright model stays shaded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum TerrainShade {
    /// Each quad's own `shade` flag, as its model bakes it.
    #[default]
    Model,
    /// No directional shading.
    Off,
    /// Every quad shaded by the level's two entity light directions.
    Entity,
}

// no wire: borrowed call-scoped view; transport uses owned snapshots rather than serializing references.
/// Faces textured by `source_sprite` drawn with `target_sprite` instead, their unit UVs
/// multiplied by `scale` then moved by `offset`: a belt's or a scrolling screen's texture.
/// Only faces of that exact sprite change.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct TerrainUvShift<'a> {
    /// The sprite the model's faces name (`mymod:block/belt`).
    pub source_sprite: &'a str,
    /// The sprite drawn in its place (`mymod:block/belt_scroll`).
    pub target_sprite: &'a str,
    /// Multiplies each UV, in sprite units (1 is the whole sprite).
    pub scale: [f32; 2],
    /// Added to each UV after `scale`, in sprite units.
    pub offset: [f32; 2],
}

impl<'a> TerrainUvShift<'a> {
    /// Faces of `source_sprite` drawn with `target_sprite`, UVs times `scale` plus `offset`.
    #[inline]
    pub fn new(
        source_sprite: &'a str,
        target_sprite: &'a str,
        scale: [f32; 2],
        offset: [f32; 2],
    ) -> Self {
        Self {
            source_sprite,
            target_sprite,
            scale,
            offset,
        }
    }
}

// no wire: borrowed call-scoped view; transport uses owned snapshots rather than serializing references.
/// How [`BlockEntityDrawContext::terrain_model`](crate::BlockEntityDrawContext::terrain_model)
/// draws a model. The default is the model's own materials and shading, its baked UVs and no
/// tint.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct TerrainModelOptions<'a> {
    /// The material every quad uses, or each quad's own.
    pub material: TerrainMaterial,
    /// Directional shading.
    pub shade: TerrainShade,
    /// One sprite drawn as another, with its UVs moved.
    pub uv_shift: Option<TerrainUvShift<'a>>,
    /// How far each face's UVs pull in towards their centre before the shift, `0.0..=1.0`:
    /// `0.0` keeps the baked UVs. A model baked by an older `FaceBakery` passes the atlas shrink
    /// ratio it was made with; a loader that doesn't shrink keeps `0.0`.
    pub uv_shrink_ratio: f32,
    /// `0xRRGGBB` multiplied into quads with a tint index; other quads keep their colour.
    pub tint: u32,
}

impl Default for TerrainModelOptions<'_> {
    fn default() -> Self {
        Self {
            material: TerrainMaterial::Model,
            shade: TerrainShade::Model,
            uv_shift: None,
            uv_shrink_ratio: 0.0,
            tint: 0xFF_FFFF,
        }
    }
}

impl<'a> TerrainModelOptions<'a> {
    /// The same options with every quad in `material`.
    #[inline]
    pub fn with_material(self, material: TerrainMaterial) -> Self {
        Self { material, ..self }
    }
    /// The same options shaded by `shade`.
    #[inline]
    pub fn with_shade(self, shade: TerrainShade) -> Self {
        Self { shade, ..self }
    }
    /// The same options with `shift` applied.
    #[inline]
    pub fn with_uv_shift(self, shift: TerrainUvShift<'a>) -> Self {
        Self {
            uv_shift: Some(shift),
            ..self
        }
    }
    /// The same options with UVs shrunk by `ratio`. The draw call checks it.
    #[inline]
    pub fn with_uv_shrink_ratio(self, ratio: f32) -> Self {
        Self {
            uv_shrink_ratio: ratio,
            ..self
        }
    }
    /// The same options with tinted quads multiplied by `0xRRGGBB` `tint`.
    #[inline]
    pub fn with_tint(self, tint: u32) -> Self {
        Self { tint, ..self }
    }
}

/// Why [`BlockEntityDrawContext::terrain_model`](crate::BlockEntityDrawContext::terrain_model)
/// drew nothing. A refused draw appends no geometry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum TerrainModelError {
    /// The handle is stale or not a resolved standalone model.
    Model(crate::ModelError),
    /// A quad could not be drawn, so none was: under [`TerrainMaterial::Model`] a translucent
    /// quad or an unknown render type; a sprite missing from the block sprites; a pose or UV
    /// that is not finite, or a pose that flattens the model; a shrink ratio outside
    /// `0.0..=1.0`; or a shift whose scale or offset is not finite, whose sprites are unknown,
    /// or whose source sprite no face of the model uses.
    Refused,
}
