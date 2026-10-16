//! The extras' HUD overlays (FPS counter, keystrokes): small boxes in the
//! vanilla font, in GUI pixels, stacked per corner (`docs/extras.md`).
use crate::{
    extras::{Corner, FpsReading},
    hud::{HudAtlas, HudBatch, Rect, rgba},
};

/// The gap between an overlay and the screen edge or the next overlay.
const MARGIN: f32 = 2.0;
const BACKGROUND: u32 = 0x7800_0000;

/// Where the next overlay goes in each corner.
pub struct Corners {
    width: f32,
    height: f32,
    /// Distance from the corner's top or bottom edge, per `Corner::ALL`.
    used: [f32; 4],
}
impl Corners {
    /// `top_right` reserves the status effect icons' rows.
    pub fn new(width: f32, height: f32, top_right: f32) -> Self {
        Self {
            width,
            height,
            used: [0.0, top_right, 0.0, 0.0],
        }
    }
    /// The top-left position of a `w` x `h` box in `corner`, below (or
    /// above) what the corner already holds.
    pub fn place(&mut self, corner: Corner, w: f32, h: f32) -> (f32, f32) {
        let i = Corner::ALL.iter().position(|c| *c == corner).unwrap_or(0);
        let edge = self.used[i] + MARGIN;
        self.used[i] = edge + h;
        let x = if corner.left() {
            MARGIN
        } else {
            self.width - MARGIN - w
        };
        let y = if corner.top() {
            edge
        } else {
            self.height - edge - h
        };
        (x, y)
    }
}

/// How the minimap shows: its corner and side in GUI pixels, its image (an
/// atlas rect), the player arrow (the map item's decoration sprite) turned
/// `arrow` degrees clockwise from up, and the rim's N toward `north` when the
/// map turns.
pub struct MinimapLook {
    pub corner: Corner,
    pub size: f32,
    pub image: Rect,
    pub arrow: f32,
    pub north: Option<(f32, f32)>,
}
pub fn draw_minimap(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    corners: &mut Corners,
    look: &MinimapLook,
) {
    let MinimapLook {
        corner,
        size,
        image,
        arrow,
        north,
    } = *look;
    let (x, y) = corners.place(corner, size, size);
    batch.rect_uv(x, y, size, size, image, u32::MAX);
    let (cx, cy) = (x + size / 2.0, y + size / 2.0);
    if let Some(r) = atlas.sprites.get("map/decorations/player").copied() {
        // MapRenderer's decorations draw the sprite flipped: its arrow
        // points down the texture, so up on screen is half a turn.
        let (s, c) = (arrow + 180.0).to_radians().sin_cos();
        let p = |dx: f32, dy: f32| [cx + dx * c - dy * s, cy + dx * s + dy * c];
        let h = 4.0;
        batch.quad(
            [p(-h, h), p(h, h), p(h, -h), p(-h, -h)],
            [[r.u0, r.v0], [r.u1, r.v0], [r.u1, r.v1], [r.u0, r.v1]],
            u32::MAX,
        );
    }
    if let Some((nx, ny)) = north {
        let reach = size / 2.0 - 6.0;
        let tw = HudBatch::text_width(atlas, "N");
        let (tx, ty) = (cx + nx * reach - tw / 2.0, cy + ny * reach - 4.0);
        batch.text(
            atlas,
            "N",
            tx.round(),
            ty.round(),
            rgba(255, 255, 255, 255),
            true,
        );
    }
}

/// The FPS counter's text.
pub fn fps_text(r: FpsReading, lows: bool) -> String {
    if lows {
        format!("{} fps  min {}  1% {}", r.fps, r.min, r.low)
    } else {
        format!("{} fps", r.fps)
    }
}
pub fn draw_fps(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    corners: &mut Corners,
    corner: Corner,
    reading: FpsReading,
    lows: bool,
) {
    let text = fps_text(reading, lows);
    let w = HudBatch::text_width(atlas, &text) + 4.0;
    let (x, y) = corners.place(corner, w, 12.0);
    batch.fill(atlas, x, y, w, 12.0, BACKGROUND);
    batch.text(
        atlas,
        &text,
        x + 2.0,
        y + 2.0,
        rgba(255, 255, 255, 255),
        true,
    );
}

/// A box of text lines (a mod's HUD panel): `(text, 0xRRGGBB)` each.
pub fn draw_lines(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    corners: &mut Corners,
    corner: Corner,
    lines: &[(&str, u32)],
) {
    if lines.is_empty() {
        return;
    }
    let w = lines
        .iter()
        .map(|(text, _)| HudBatch::text_width(atlas, text))
        .fold(0.0, f32::max)
        + 4.0;
    let h = 10.0 * lines.len() as f32 + 2.0;
    let (x, y) = corners.place(corner, w, h);
    batch.fill(atlas, x, y, w, h, BACKGROUND);
    for (i, (text, color)) in lines.iter().enumerate() {
        let [_, r, g, b] = color.to_be_bytes();
        let y = y + 2.0 + 10.0 * i as f32;
        batch.text(atlas, text, x + 2.0, y, rgba(r, g, b, 255), true);
    }
}

/// What the keystroke overlay shows.
pub struct Keystrokes {
    /// Forward, left, back, right: held, and the bound key's name.
    pub moves: [(bool, String); 4],
    pub jump: bool,
    pub attack: bool,
    pub use_: bool,
    /// Clicks per second of the left and right buttons, when shown.
    pub cps: Option<(usize, usize)>,
}
const KEY: f32 = 22.0;
const GAP: f32 = 2.0;
/// The overlay's size.
pub fn keystrokes_size() -> (f32, f32) {
    (3.0 * KEY + 2.0 * GAP, 3.0 * KEY + 3.0 * GAP + 10.0)
}
fn key_box(batch: &mut HudBatch, atlas: &HudAtlas, rect: [f32; 4], held: bool, lines: &[&str]) {
    let [x, y, w, h] = rect;
    let (fill, ink) = if held {
        (rgba(255, 255, 255, 176), rgba(20, 20, 20, 255))
    } else {
        (BACKGROUND, rgba(255, 255, 255, 255))
    };
    batch.fill(atlas, x, y, w, h, fill);
    let total = lines.len() as f32 * 9.0 - 1.0;
    for (i, line) in lines.iter().enumerate() {
        let tw = HudBatch::text_width(atlas, line);
        let tx = (x + (w - tw) / 2.0).round();
        let ty = (y + (h - total) / 2.0).round() + i as f32 * 9.0;
        batch.text(atlas, line, tx, ty, ink, !held);
    }
}
pub fn draw_keystrokes(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    corners: &mut Corners,
    corner: Corner,
    k: &Keystrokes,
) {
    let (w, h) = keystrokes_size();
    let (x, y) = corners.place(corner, w, h);
    let step = KEY + GAP;
    let [forward, left, back, right] = &k.moves;
    key_box(
        batch,
        atlas,
        [x + step, y, KEY, KEY],
        forward.0,
        &[&forward.1],
    );
    for (i, key) in [left, back, right].into_iter().enumerate() {
        key_box(
            batch,
            atlas,
            [x + i as f32 * step, y + step, KEY, KEY],
            key.0,
            &[&key.1],
        );
    }
    let mw = (w - GAP) / 2.0;
    let my = y + 2.0 * step;
    for (i, (held, label, cps)) in [
        (k.attack, "LMB", k.cps.map(|c| c.0)),
        (k.use_, "RMB", k.cps.map(|c| c.1)),
    ]
    .into_iter()
    .enumerate()
    {
        let rate = cps.map(|n| format!("{n} CPS"));
        let lines: Vec<&str> = std::iter::once(label).chain(rate.as_deref()).collect();
        key_box(
            batch,
            atlas,
            [x + i as f32 * (mw + GAP), my, mw, KEY],
            held,
            &lines,
        );
    }
    // The space bar: a line in a short key.
    let sy = my + step;
    key_box(batch, atlas, [x, sy, w, 10.0], k.jump, &[]);
    let ink = if k.jump {
        rgba(20, 20, 20, 255)
    } else {
        rgba(255, 255, 255, 255)
    };
    batch.fill(atlas, x + w / 2.0 - 12.0, sy + 4.0, 24.0, 2.0, ink);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corners_stack_away_from_their_edge() {
        let mut c = Corners::new(400.0, 300.0, 26.0);
        assert_eq!(c.place(Corner::TopLeft, 50.0, 12.0), (2.0, 2.0));
        assert_eq!(c.place(Corner::TopLeft, 70.0, 20.0), (2.0, 16.0));
        // Top right starts under the effect icons.
        assert_eq!(c.place(Corner::TopRight, 50.0, 12.0), (348.0, 28.0));
        assert_eq!(c.place(Corner::BottomRight, 50.0, 12.0), (348.0, 286.0));
        assert_eq!(c.place(Corner::BottomRight, 50.0, 10.0), (348.0, 274.0));
        assert_eq!(c.place(Corner::BottomLeft, 10.0, 10.0), (2.0, 288.0));
    }

    #[test]
    fn fps_text_with_and_without_lows() {
        let r = FpsReading {
            fps: 144,
            min: 61,
            low: 98,
        };
        assert_eq!(fps_text(r, false), "144 fps");
        assert_eq!(fps_text(r, true), "144 fps  min 61  1% 98");
    }
}
