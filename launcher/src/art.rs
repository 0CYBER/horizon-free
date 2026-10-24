//! The launcher's pixel art, drawn here rather than shipped: the stone texture behind the pages
//! and the landscape behind Home (the vanilla grass block is `browser::images::grass_block`).
//! GPUI samples images smoothly, so every image is baked at the window's scale with each texel a
//! whole block of device pixels, and drawn at exactly its size: the edges stay sharp. Each image
//! is made once per appearance and scale, then reused; Home's landscape is made again only when
//! the window grows wider than it.
use gpui::{RenderImage, Window};
use horizon_theme as tokens;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Default)]
pub struct Art {
    /// The stone texture for each appearance and scale.
    stones: HashMap<(bool, u32), Arc<RenderImage>>,
    /// Home's landscape: its appearance and scale, and its width in texels.
    scene: Option<((bool, u32), usize, Arc<RenderImage>)>,
    /// Landscapes replaced since the last frame, still in the window's sprite atlas.
    replaced: Vec<Arc<RenderImage>>,
}

/// The stone texture comes in squares of this many logical pixels (4 by 4 tiles).
pub const STONE: f32 = tokens::TILE_TEXEL * (tokens::TILE_TEXELS * 4) as f32;
/// Home's landscape: texels of this many logical pixels, and its height in texels. Its width
/// grows in steps of `SCENE_STEP` texels, so resizing a window rarely makes another.
pub const SCENE_TEXEL: f32 = 6.;
pub const SCENE_HEIGHT: usize = 56;
const SCENE_STEP: usize = 64;

impl Art {
    /// The page's stone texture, for a window at `scale`.
    pub fn stone(&mut self, page: u32, dark: bool, scale: f32) -> Arc<RenderImage> {
        self.stones
            .entry((dark, scale_key(scale)))
            .or_insert_with(|| {
                let n = tokens::TILE_TEXELS * 4;
                bake(&opaque(&stone(page, n)), n, n, tokens::TILE_TEXEL, scale)
            })
            .clone()
    }

    /// Home's landscape, at least `width` logical pixels wide (the banner clips the rest), and
    /// its width: a sunset in the dark appearance, a dawn in the light one.
    pub fn scene(&mut self, dark: bool, scale: f32, width: f32) -> (Arc<RenderImage>, f32) {
        let key = (dark, scale_key(scale));
        let texels = (width / SCENE_TEXEL).ceil().max(1.) as usize;
        if !matches!(&self.scene, Some((k, w, _)) if *k == key && *w >= texels) {
            let w = texels.next_multiple_of(SCENE_STEP);
            let palette = if dark { &SUNSET } else { &DAWN };
            let texels = opaque(&scene(palette, 3, w, SCENE_HEIGHT));
            let image = bake(&texels, w, SCENE_HEIGHT, SCENE_TEXEL, scale);
            if let Some((_, _, old)) = self.scene.replace((key, w, image)) {
                self.replaced.push(old);
            }
        }
        let (_, w, image) = self.scene.as_ref().expect("made above");
        (image.clone(), *w as f32 * SCENE_TEXEL)
    }

    /// Frees the landscapes replaced since the last frame: GPUI keeps an image in the window's
    /// sprite atlas until it is dropped there.
    pub fn free_replaced(&mut self, window: &mut Window) {
        for image in self.replaced.drain(..) {
            let _ = window.drop_image(image);
        }
    }
}

/// `0xRRGGBB` texels, opaque.
fn opaque(texels: &[u32]) -> Vec<[u8; 4]> {
    texels
        .iter()
        .map(|c| [(c >> 16) as u8, (c >> 8) as u8, *c as u8, 255])
        .collect()
}

/// The colour at the top of Home's sky, for the banner above the landscape when it is taller.
pub fn sky_top(dark: bool) -> u32 {
    if dark {
        SUNSET.sky[0]
    } else {
        DAWN.sky[0]
    }
}

fn scale_key(scale: f32) -> u32 {
    (scale * 100.).round() as u32
}

/// `texels` (RGBA, row by row) as an image whose texels are `texel` logical pixels square at
/// `scale`. Texel edges fall on whole device pixels where the scale allows (1×, 2×, 3×).
pub fn bake(texels: &[[u8; 4]], w: usize, h: usize, texel: f32, scale: f32) -> Arc<RenderImage> {
    let size = |n: usize| ((n as f32 * texel * scale).round() as u32).max(1);
    let (dw, dh) = (size(w), size(h));
    let mut buf = image::RgbaImage::new(dw, dh);
    for (y, row) in buf.rows_mut().enumerate() {
        let ty = (y * h / dh as usize).min(h - 1);
        for (x, px) in row.enumerate() {
            let tx = (x * w / dw as usize).min(w - 1);
            let [r, g, b, a] = texels[ty * w + tx];
            // GPUI wants BGRA, its alpha premultiplied.
            let pre = |c: u8| (c as u32 * a as u32 / 255) as u8;
            *px = image::Rgba([pre(b), pre(g), pre(r), a]);
        }
    }
    Arc::new(RenderImage::new([image::Frame::new(buf)]))
}

/// A small, fixed random sequence (xorshift), so the art is the same on every start.
struct Rng(u32);

impl Rng {
    fn next(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        (x >> 8) as f32 / (1 << 24) as f32
    }
}

fn shade(c: u32, by: i16) -> u32 {
    [16, 8, 0].iter().fold(0, |out, s| {
        let v = (((c >> s) & 255) as i16 + by).clamp(0, 255) as u32;
        out | v << s
    })
}

fn mix(a: u32, b: u32, t: f32) -> u32 {
    [16, 8, 0].iter().fold(0, |out, s| {
        let (x, y) = (((a >> s) & 255) as f32, ((b >> s) & 255) as f32);
        out | ((x + (y - x) * t).round() as u32) << s
    })
}

/// `n` by `n` texels of stone: the page colour in a few close shades, with darker flecks.
fn stone(page: u32, n: usize) -> Vec<u32> {
    let shades = tokens::TILE_SHADES.map(|by| shade(page, by));
    let mut rng = Rng(0x9e37_79b9);
    let mut out: Vec<u32> = (0..n * n)
        .map(|_| {
            let v = rng.next();
            shades[if v < 0.5 {
                0
            } else if v < 0.78 {
                1
            } else if v < 0.93 {
                2
            } else {
                3
            }]
        })
        .collect();
    for _ in 0..n * n / 50 {
        let (x, y) = (
            (rng.next() * n as f32) as usize,
            (rng.next() * n as f32) as usize,
        );
        let len = 1 + (rng.next() * 3.) as usize;
        for i in 0..len {
            out[y * n + (x + i) % n] = shades[4];
            out[((y + 1) % n) * n + (x + i + 1) % n] = shades[4];
        }
    }
    out
}

/// A landscape's colours: the sky from the top down, the sun's bands, then the hills from far
/// to near, the near hill's top edge, and the trees.
pub struct Palette {
    sky: &'static [u32],
    sun: [u32; 3],
    far: u32,
    mid: u32,
    near: u32,
    top: u32,
    trees: u32,
}

/// The sunset of Horizon's first icon: night blue to violet to coral, and a sun in three bands.
pub const SUNSET: Palette = Palette {
    sky: &[
        0x0a0f2c, 0x26206a, 0x4b2a7e, 0x7c3a88, 0xc4527a, 0xee6e5a, 0xffb25e,
    ],
    sun: [0xfff6cf, 0xffd272, 0xff9a44],
    far: 0x4a2f7e,
    mid: 0x2b1f5e,
    near: 0x151a45,
    top: 0x3d2f80,
    trees: 0x0e1236,
};

/// The light appearance's morning: lavender to peach, soft violet hills.
pub const DAWN: Palette = Palette {
    sky: &[
        0x8d7fd8, 0xa596e2, 0xbcaeea, 0xd2c4f0, 0xe7d2ee, 0xf6dce4, 0xffe6d6,
    ],
    sun: [0xffffff, 0xfff1d0, 0xffd9a8],
    far: 0xa898dc,
    mid: 0x8676c8,
    near: 0x6a5ab4,
    top: 0x9a8ae0,
    trees: 0x4c3e94,
};

/// A blocky landscape, `w` by `h` texels: banded sky, a sun half set behind the far hills and cut
/// into slices like Horizon's mark, flat clouds, three ridges of hills and a few square trees.
/// Each part draws from its own generator from left to right, so a wider landscape only adds
/// columns: the first `w` are the same at every width, and a resized window keeps its view.
pub fn scene(p: &Palette, seed: u32, w: usize, h: usize) -> Vec<u32> {
    let mut seeds = Rng(seed.wrapping_mul(0x2545_f491) | 1);
    let mut rng = || {
        seeds.next();
        Rng(seeds.0)
    };
    let mut out = vec![0; w * h];
    let horizon = h * 62 / 100;
    let band = horizon as f32 / p.sky.len() as f32;
    for y in 0..h {
        let f = y as f32 / band;
        let i = (f as usize).min(p.sky.len() - 1);
        let dither = f - i as f32 > 0.88 && i + 1 < p.sky.len();
        for x in 0..w {
            out[y * w + x] = if dither && (x + y) % 2 == 1 {
                p.sky[i + 1]
            } else {
                p.sky[i]
            };
        }
    }
    let rect = |out: &mut Vec<u32>, x: isize, y: isize, rw: usize, rh: usize, c: u32| {
        for yy in y.max(0) as usize..((y + rh as isize).max(0) as usize).min(h) {
            for xx in x.max(0) as usize..((x + rw as isize).max(0) as usize).min(w) {
                out[yy * w + xx] = c;
            }
        }
    };
    // Four clouds in every stretch of `CLOUDS` texels.
    const CLOUDS: usize = 220;
    let mut clouds = rng();
    for stretch in 0..w.div_ceil(CLOUDS) {
        for _ in 0..4 {
            let cx = (stretch * CLOUDS) as isize + (clouds.next() * CLOUDS as f32) as isize;
            let cy = (horizon as f32 * (0.12 + clouds.next() * 0.4)) as isize;
            let cw = 10 + (clouds.next() * 14.) as usize;
            let under = p.sky[((cy as f32 / band) as usize + 1).min(p.sky.len() - 1)];
            let c = mix(under, 0xffffff, 0.12);
            rect(&mut out, cx, cy, cw, 2, c);
            rect(&mut out, cx + 3, cy - 2, cw.saturating_sub(6), 2, c);
        }
    }
    // The sun, where it was when the landscape had one width (two thirds of 220 texels).
    let r = (h as f32 * 0.2) as isize;
    let (cx, cy) = (149, horizon as isize - (h as f32 * 0.11) as isize);
    for dy in -r..=0 {
        let half = ((r * r - dy * dy) as f32).sqrt().round() as isize;
        let t = (dy + r) as f32 / r as f32;
        let c = p.sun[if t < 0.4 {
            0
        } else if t < 0.75 {
            1
        } else {
            2
        }];
        if (cy + dy) % 5 != 0 || (dy as f32) < -r as f32 * 0.6 {
            rect(&mut out, cx - half, cy + dy, (half * 2) as usize, 1, c);
        }
    }
    let ridge = |mut rng: Rng, base: f32, amp: f32, rough: f32| -> Vec<usize> {
        let (mut y, mut v) = (base, 0.);
        (0..w)
            .map(|x| {
                if x % 2 == 0 {
                    v = (v + (rng.next() - 0.5) * rough) * 0.86;
                    y += v + (base - y) * 0.06;
                }
                y.clamp(base - amp, base + amp).round().max(0.) as usize
            })
            .collect()
    };
    let hf = h as f32;
    let layers = [
        (p.far, horizon as f32 - hf * 0.1, hf * 0.12, 3.2, None),
        (p.mid, horizon as f32 + hf * 0.02, hf * 0.09, 2.6, None),
        (
            p.near,
            horizon as f32 + hf * 0.18,
            hf * 0.07,
            2.2,
            Some(p.top),
        ),
    ];
    let mut near = Vec::new();
    for (c, base, amp, rough, top) in layers {
        let hs = ridge(rng(), base, amp, rough);
        for (x, &y) in hs.iter().enumerate() {
            for yy in y.min(h)..h {
                out[yy * w + x] = c;
            }
            if let (Some(top), true) = (top, y < h) {
                out[y * w + x] = top;
            }
        }
        near = hs;
    }
    // A tree somewhere in every 26 texels of the near hills.
    let mut trees = rng();
    for cell in 0..w.div_ceil(26) {
        let tx = cell * 26 + 2 + (trees.next() * 22.) as usize;
        let Some(&ty) = near.get(tx) else { break };
        let (tx, ty) = (tx as isize, ty as isize);
        rect(&mut out, tx - 2, ty - 8, 5, 5, p.trees);
        rect(&mut out, tx - 1, ty - 10, 3, 2, p.trees);
        rect(&mut out, tx, ty - 3, 1, 3, p.trees);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn art_is_the_same_on_every_start() {
        assert_eq!(scene(&SUNSET, 3, 40, 20), scene(&SUNSET, 3, 40, 20));
        assert_eq!(stone(0x0f0d17, 16), stone(0x0f0d17, 16));
    }

    #[test]
    fn the_stone_stays_within_its_shades() {
        let shades = tokens::TILE_SHADES.map(|by| shade(0x0f0d17, by));
        assert!(stone(0x0f0d17, 64).iter().all(|c| shades.contains(c)));
    }

    #[test]
    fn texels_cover_whole_device_pixels() {
        let image = bake(&opaque(&[0x112233; 4]), 2, 2, 4., 2.);
        let size = image.size(0);
        assert_eq!((size.width.0, size.height.0), (16, 16));
    }

    #[test]
    fn the_landscape_covers_the_banner_and_is_made_again_only_when_it_grows() {
        let mut art = Art::default();
        let stone = art.stone(0x0f0d17, true, 2.);
        let (first, drawn) = art.scene(true, 2., 700.);
        assert!(drawn >= 700.);
        assert_eq!(first.size(0).width.0, (drawn * 2.) as i32);
        assert_eq!(
            first.size(0).height.0,
            (SCENE_HEIGHT as f32 * SCENE_TEXEL * 2.) as i32
        );
        // A step wide: a few pixels more or a narrower window keep it.
        for width in [drawn, 400.] {
            assert!(Arc::ptr_eq(&first, &art.scene(true, 2., width).0));
        }
        let (wider, wider_drawn) = art.scene(true, 2., 1900.);
        assert!(!Arc::ptr_eq(&first, &wider) && wider_drawn >= 1900.);
        assert!(Arc::ptr_eq(&art.replaced[0], &first));
        assert!(Arc::ptr_eq(&stone, &art.stone(0x0f0d17, true, 2.)));
    }

    #[test]
    fn a_wider_landscape_only_adds_columns() {
        let (h, narrow, wide) = (SCENE_HEIGHT, 150, 400);
        let a = scene(&SUNSET, 3, narrow, h);
        let b = scene(&SUNSET, 3, wide, h);
        for y in 0..h {
            assert_eq!(
                a[y * narrow..(y + 1) * narrow],
                b[y * wide..y * wide + narrow],
                "row {y}"
            );
        }
    }
}
