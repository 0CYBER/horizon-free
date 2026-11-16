//! The dying dragon's dissolve (`EnderDragonRenderer`).
use super::*;

/// The dying model's colour, `ARGB.white(1 - deathTime / 200)`: `Mth.floor` of the alpha
/// shifted as a Java int, so past 200 it wraps as vanilla's does.
pub(super) fn dissolve_color(death: f32) -> u32 {
    let alpha = ((1.0 - death / 200.0) * 255.0).floor() as i32;
    (alpha << 24) as u32 | 0x00FF_FFFF
}
/// `DissolveMaskSampler` at the vertex's `UV0`: the same place in the mask's sprite as in the
/// skin's, as the dissolve overlay reads it from `glint_uv` (entity.wgsl).
pub(super) fn dissolve_mask(quads: &mut [Vertex], skin: [f32; 4], mask: [f32; 4]) {
    for v in quads {
        let u = (v.uv[0] - skin[0]) / (skin[2] - skin[0]);
        let w = (v.uv[1] - skin[1]) / (skin[3] - skin[1]);
        v.glint_uv = [
            mask[0] + (mask[2] - mask[0]) * u,
            mask[1] + (mask[3] - mask[1]) * w,
        ];
    }
}
