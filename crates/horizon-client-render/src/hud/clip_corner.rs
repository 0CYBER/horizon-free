//! Preserve every interpolated face attribute when a GUI clip cuts its corner.
use super::HudVertex;

pub(super) fn clip_corner(
    [tl, tr, br, bl]: [HudVertex; 4],
    [x, y]: [f32; 2],
    [s, r]: [f32; 2],
) -> HudVertex {
    let mix = |a: f32, b: f32, c: f32, d: f32| {
        let top = a + (b - a) * s;
        let bottom = d + (c - d) * s;
        top + (bottom - top) * r
    };
    let mut v = tl;
    v.pos = [x, y];
    v.uv = std::array::from_fn(|i| mix(tl.uv[i], tr.uv[i], br.uv[i], bl.uv[i]));
    v.glint_uv = std::array::from_fn(|i| {
        mix(
            tl.glint_uv[i],
            tr.glint_uv[i],
            br.glint_uv[i],
            bl.glint_uv[i],
        )
    });
    let top = crate::hud_kit::shapes::mix_colour(tl.color, tr.color, s);
    let bottom = crate::hud_kit::shapes::mix_colour(bl.color, br.color, s);
    v.color = crate::hud_kit::shapes::mix_colour(top, bottom, r);
    v.depth = mix(tl.depth, tr.depth, br.depth, bl.depth);
    v
}
