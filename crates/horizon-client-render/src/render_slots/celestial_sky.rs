//! Native-only renderer slot for typed celestial geometry. It carries no GPU handles.

use glam::Mat4;
use mod_api::AtlasRegion;
use std::sync::OnceLock;

/// Shared triangle budget for the main world's celestial layer, per frame.
pub const MAX_CELESTIAL_TRIANGLES: usize = 4096;

/// A corner of a celestial triangle, in the same axes as the world but relative to the camera.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct CelestialVertex {
    /// Each offset component must be in -512..=512 blocks.
    pub position: [f32; 3],
    /// UV in 0..=1 inside the submitted sprite, top-left at `[0, 0]`.
    pub uv: [f32; 2],
    /// Unlit tint in vanilla gamma (RGB 0..=16, paper white at 1); alpha is coverage (0..=1).
    pub color: [f32; 4],
}

/// A refused batch contributes no triangles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CelestialDrawError {
    IncompleteTriangle,
    Capacity,
    InvalidAtlasRegion,
    InvalidVertex,
}

/// The camera and triangle sink for the main view's visible Overworld-style sky.
#[non_exhaustive]
pub struct CelestialSky<'a> {
    /// Camera-relative world to clip space, including the vanilla view effects and jitter.
    pub view_proj: Mat4,
    /// Vanilla's star X rotation in degrees, including when the stars themselves do not draw.
    pub star_angle_degrees: f32,
    // Validated vertices with atlas UVs, instead of sprite-local UVs.
    vertices: &'a mut Vec<CelestialVertex>,
}

impl<'a> CelestialSky<'a> {
    pub(crate) fn new(
        view_proj: Mat4,
        star_angle_degrees: f32,
        vertices: &'a mut Vec<CelestialVertex>,
    ) -> Self {
        Self {
            view_proj,
            star_angle_degrees,
            vertices,
        }
    }

    /// Vanilla's star space to camera-relative world, with no camera translation. Transform
    /// vertices by this rotation before submitting if they should turn with the stars.
    pub fn star_rotation(&self) -> Mat4 {
        Mat4::from_rotation_y((-90.0_f32).to_radians())
            * Mat4::from_rotation_x(self.star_angle_degrees.to_radians())
    }

    /// Append a triangle list, in either winding, using one GUI atlas sprite. Resolve the
    /// `AtlasRegion` again following each GUI atlas stitch. Batches draw in submission order.
    pub fn draw_triangles(
        &mut self,
        sprite: AtlasRegion,
        vertices: &[CelestialVertex],
    ) -> Result<(), CelestialDrawError> {
        use CelestialDrawError::*;

        if !vertices.len().is_multiple_of(3) {
            return Err(IncompleteTriangle);
        }
        if vertices.len() > MAX_CELESTIAL_TRIANGLES * 3 - self.vertices.len() {
            return Err(Capacity);
        }
        let uv_valid = |uv: f32| (0.0..=1.0).contains(&uv);
        let [u0, v0, u1, v1] = sprite.uv;
        if sprite.size.contains(&0) || !sprite.uv.into_iter().all(uv_valid) || u0 >= u1 || v0 >= v1
        {
            return Err(InvalidAtlasRegion);
        }
        let (du, dv) = (u1 - u0, v1 - v0);
        let start = self.vertices.len();
        self.vertices.reserve(vertices.len());
        for &v in vertices {
            if !v.position.iter().all(|p| (-512.0..=512.0).contains(p))
                || !v.uv.into_iter().all(uv_valid)
                || !v.color[..3].iter().all(|c| (0.0..=16.0).contains(c))
                || !uv_valid(v.color[3])
            {
                self.vertices.truncate(start);
                return Err(InvalidVertex);
            }
            self.vertices.push(CelestialVertex {
                uv: [u0 + v.uv[0] * du, v0 + v.uv[1] * dv],
                ..v
            });
        }
        Ok(())
    }
}

/// The native provider's installed celestial-stage callback type.
pub type Draw = for<'a> fn(&mut CelestialSky<'a>);
static SELECT: OnceLock<fn() -> Option<Draw>> = OnceLock::new();

/// Install the native celestial-stage selector before constructing the renderer. With no
/// listener it returns `None` and the renderer needs no celestial-layer resources.
pub fn install(select: fn() -> Option<Draw>) {
    let _ = SELECT.set(select);
}

pub(crate) fn listener() -> Option<Draw> {
    SELECT.get().and_then(|select| select())
}
