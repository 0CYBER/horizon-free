//! Horizon's own menu style ("Menu style: Horizon" in the options), shared by
//! the front-end screens: title, world list, create world, multiplayer,
//! options and pause. Everything vanilla compares (HUD, inventories, chat,
//! the pack and key-bind screens) keeps the classic widgets in `ui`.
//!
//! It is the launcher's look, from the same tokens: `crates/horizon-theme`
//! holds the palette, type scale, spacing, radii, control sizes, motion and
//! the bundled fonts, and `launcher/src/theme.rs` reads the same values, so
//! changing one changes both. The components below port the launcher's
//! (`launcher/src/main.rs`): button with hover layer and press state, sidebar
//! list with the sliding selection, segmented control, switch, settings rows,
//! page header, and its `ease_out_quint` tweens over `QUICK` and `MOVE`.
//!
//! How to use it from a screen (or to add options, e.g. the Simple/Advanced
//! graphics settings): build a [`Ui`] over the frame's `HudBatch` with
//! [`Ui::new`], lay out in dp (the launcher's logical pixels) with the widgets below, and give every interactive widget a stable
//! `id`. Clicks run the same layout with [`Ui::dry`] and read [`Ui::hit`]: one
//! function draws and hit-tests, so they never disagree. The options screen
//! (`options_ui::OptionsUi::draw_horizon`) lists its sections in
//! `options_ui::HORIZON_SECTIONS`; a section is a title, a short blurb and its
//! rows (`HRow`), so new settings only need rows.
use crate::hud::{HudAtlas, HudBatch, Rect};
use ab_glyph::{Font as _, FontRef, PxScale, ScaleFont as _};
use std::{
    cell::RefCell,
    collections::HashMap,
    path::Path,
    sync::atomic::{AtomicU8, Ordering},
    time::Instant,
};

/// Which drawing the front-end screens use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuStyle {
    Horizon,
    /// Vanilla's screens, exactly as the parity harnesses expect them.
    Classic,
}
static STYLE: AtomicU8 = AtomicU8::new(0);
pub fn style() -> MenuStyle {
    if STYLE.load(Ordering::Relaxed) == 1 {
        MenuStyle::Classic
    } else {
        MenuStyle::Horizon
    }
}
pub fn horizon() -> bool {
    style() == MenuStyle::Horizon
}
pub fn set_style(s: MenuStyle) {
    STYLE.store(u8::from(s == MenuStyle::Classic), Ordering::Relaxed);
}
/// Horizon-only preferences live in `horizon.txt` beside `options.txt`, so
/// vanilla's options file stays exactly as vanilla writes it.
pub fn load_preferences(dir: &Path) {
    let text = std::fs::read_to_string(dir.join("horizon.txt")).unwrap_or_default();
    let classic = text.lines().any(|l| l.trim() == "menuStyle:classic");
    // HORIZON_MENU_STYLE=classic forces vanilla's screens (tests, harnesses).
    let forced = std::env::var("HORIZON_MENU_STYLE").ok();
    set_style(match forced.as_deref() {
        Some("classic") => MenuStyle::Classic,
        Some("horizon") => MenuStyle::Horizon,
        _ if classic => MenuStyle::Classic,
        _ => MenuStyle::Horizon,
    });
}
pub fn save_preferences(dir: &Path) {
    let value = match style() {
        MenuStyle::Horizon => "horizon",
        MenuStyle::Classic => "classic",
    };
    let _ = std::fs::write(dir.join("horizon.txt"), format!("menuStyle:{value}\n"));
}

/// The launcher's dark palette (`horizon_theme::DARK`) in the HUD's colour
/// encoding, plus the translucent surfaces the menus put over the backdrop.
pub mod color {
    use crate::hud::rgba;
    use horizon_theme::DARK as P;
    pub const fn hex(c: u32, a: u8) -> u32 {
        (a as u32) << 24 | (c & 0xff) << 16 | (c >> 8 & 0xff) << 8 | (c >> 16 & 0xff)
    }
    pub const BG: u32 = hex(P.bg, 255);
    pub const SIDEBAR: u32 = hex(P.sidebar, 255);
    pub const RAISED: u32 = hex(P.raised, 255);
    pub const HOVER: u32 = hex(P.hover, 255);
    pub const BORDER: u32 = hex(P.border, 255);
    pub const TEXT: u32 = hex(P.text, 255);
    pub const MUTED: u32 = hex(P.muted, 255);
    pub const FAINT: u32 = hex(P.faint, 255);
    pub const ACCENT: u32 = hex(P.accent, 255);
    pub const ON_ACCENT: u32 = hex(P.on_accent, 255);
    pub const DANGER: u32 = hex(P.danger, 255);
    pub const WHITE: u32 = hex(0xffffff, 255);
    pub fn with_alpha(c: u32, a: f32) -> u32 {
        let base = (c >> 24) as f32;
        (c & 0x00ff_ffff) | ((base * a.clamp(0.0, 1.0)).round() as u32) << 24
    }
    /// Linear blend of two colours (with alpha).
    pub fn mix(a: u32, b: u32, t: f32) -> u32 {
        let t = t.clamp(0.0, 1.0);
        let ch = |c: u32, s: u32| ((c >> s) & 255) as f32;
        let m = |s: u32| (ch(a, s) + (ch(b, s) - ch(a, s)) * t).round() as u8;
        rgba(m(0), m(8), m(16), m(24))
    }
}

/// The launcher's type scale and shapes (`horizon_theme`), in dp.
pub use horizon_theme::{
    BODY, CONTROL, CONTROL_LARGE, HERO, LARGE, RADIUS, RADIUS_CARD, RADIUS_GROUP, RADIUS_SEGMENT,
    ROW_LABEL, SEGMENT, SIDEBAR, SMALL, TITLE, WORDMARK, ease_out_cubic, ease_out_quint,
};
/// Motion durations in seconds (`horizon_theme::QUICK` and `MOVE`).
pub fn quick() -> f32 {
    horizon_theme::QUICK.as_secs_f32()
}
pub fn move_() -> f32 {
    horizon_theme::MOVE.as_secs_f32()
}
/// How visible the live title backdrop is (0..1), set by the game each frame.
static BACKDROP: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
pub fn set_backdrop(fade: f32) {
    BACKDROP.store(fade.to_bits(), Ordering::Relaxed);
}
pub fn backdrop() -> f32 {
    f32::from_bits(BACKDROP.load(Ordering::Relaxed))
}
/// The window size of the last drawn frame, for hit-testing passes.
static WINDOW: std::sync::Mutex<[u32; 2]> = std::sync::Mutex::new([1280, 800]);
/// An empty batch at the last frame's window size (for [`Ui::dry`]).
pub fn scratch_batch() -> HudBatch {
    HudBatch::new(*WINDOW.lock().unwrap_or_else(|e| e.into_inner()))
}
/// Whether the left mouse button is held (the primary button's press state).
static PRESSED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
pub fn set_pressed(down: bool) {
    PRESSED.store(down, Ordering::Relaxed);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Face {
    Display,
    Regular,
    Medium,
    Semibold,
    Mono,
}
struct Fonts {
    faces: [FontRef<'static>; 5],
}
impl Fonts {
    fn load() -> Self {
        let f = |b: &'static [u8]| FontRef::try_from_slice(b).expect("bundled font");
        Self {
            faces: [
                f(horizon_theme::fonts::INSTRUMENT_SERIF),
                f(horizon_theme::fonts::GEIST_REGULAR),
                f(horizon_theme::fonts::GEIST_MEDIUM),
                f(horizon_theme::fonts::GEIST_SEMIBOLD),
                f(horizon_theme::fonts::GEIST_MONO),
            ],
        }
    }
    fn face(&self, f: Face) -> &FontRef<'static> {
        &self.faces[f as usize]
    }
    /// The ab_glyph scale for an em size in pixels.
    fn scale(&self, f: Face, px: f32) -> PxScale {
        let font = self.face(f);
        let upem = font.units_per_em().unwrap_or(1000.0);
        PxScale::from(px * font.height_unscaled() / upem)
    }
}
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Key {
    Glyph(Face, u16, u16),
    /// A quarter disc (`false`) or a 1px quarter ring of this radius in pixels.
    Corner(u16, bool),
}
#[derive(Clone, Copy)]
struct Cached {
    rect: Option<Rect>,
    /// Offset of the bitmap from the pen (glyphs) in pixels.
    left: f32,
    top: f32,
}
/// Glyphs and corner masks rasterized into the atlas region `horizon/ui-text`.
#[derive(Clone, Copy)]
struct Tween {
    from: f32,
    to: f32,
    start: Instant,
    secs: f32,
}
impl Tween {
    fn value(&self) -> f32 {
        let t = if self.secs <= 0.0 {
            1.0
        } else {
            self.start.elapsed().as_secs_f32() / self.secs
        };
        self.from + (self.to - self.from) * ease_out_quint(t)
    }
}
struct Cache {
    fonts: Fonts,
    entries: HashMap<Key, Cached>,
    shelf: (u32, u32, u32),
    pending: Vec<crate::font::Patch>,
    /// The atlas the entries were uploaded to (a new atlas starts over).
    atlas: usize,
    last_frame: Option<Instant>,
    /// Tweens by widget id: from, to, start, duration (hover, selection, pages).
    anims: HashMap<u64, Tween>,
    dt: f32,
}
thread_local! {
    static CACHE: RefCell<Option<Cache>> = const { RefCell::new(None) };
}
fn with_cache<R>(atlas: &HudAtlas, f: impl FnOnce(&mut Cache) -> R) -> R {
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        let id = atlas as *const HudAtlas as usize;
        let cache = c.get_or_insert_with(|| Cache {
            fonts: Fonts::load(),
            entries: HashMap::new(),
            shelf: (0, 0, 0),
            pending: Vec::new(),
            atlas: id,
            last_frame: None,
            anims: HashMap::new(),
            dt: 0.0,
        });
        if cache.atlas != id {
            cache.atlas = id;
            cache.entries.clear();
            cache.shelf = (0, 0, 0);
            cache.pending.clear();
        }
        f(cache)
    })
}
impl Cache {
    fn region(atlas: &HudAtlas) -> Option<Rect> {
        atlas.sprites.get("horizon/ui-text").copied()
    }
    /// Reserve `w x h` pixels; on overflow forget everything and start over.
    fn place(&mut self, atlas: &HudAtlas, w: u32, h: u32) -> Option<(u32, u32, Rect)> {
        let region = Self::region(atlas)?;
        let (mut x, mut y, mut row) = self.shelf;
        if x + w + 1 > region.w {
            x = 0;
            y += row + 1;
            row = 0;
        }
        if y + h > region.h || w > region.w {
            self.entries.clear();
            self.shelf = (0, 0, 0);
            return None;
        }
        self.shelf = (x + w + 1, y, row.max(h));
        Some((x, y, region))
    }
    fn upload(&mut self, atlas: &HudAtlas, w: u32, h: u32, alpha: &[f32]) -> Option<Rect> {
        let (x, y, region) = self.place(atlas, w, h)?;
        let (ax, ay) = (
            (region.u0 * atlas.width as f32).round() as u32 + x,
            (region.v0 * atlas.height as f32).round() as u32 + y,
        );
        let rgba = alpha
            .iter()
            .flat_map(|a| [255, 255, 255, (a.clamp(0.0, 1.0) * 255.0).round() as u8])
            .collect();
        self.pending.push(crate::font::Patch {
            x: ax,
            y: ay,
            width: w,
            height: h,
            rgba,
        });
        let (aw, ah) = (atlas.width as f32, atlas.height as f32);
        Some(Rect {
            u0: ax as f32 / aw,
            v0: ay as f32 / ah,
            u1: (ax + w) as f32 / aw,
            v1: (ay + h) as f32 / ah,
            w,
            h,
        })
    }
    fn glyph(&mut self, atlas: &HudAtlas, face: Face, id: ab_glyph::GlyphId, px: f32) -> Cached {
        let size = (px * 4.0).round() as u16;
        let key = Key::Glyph(face, id.0, size);
        if let Some(c) = self.entries.get(&key) {
            return *c;
        }
        let scale = self.fonts.scale(face, f32::from(size) / 4.0);
        let glyph = id.with_scale_and_position(scale, ab_glyph::point(0.0, 0.0));
        let made = match self.fonts.face(face).outline_glyph(glyph) {
            Some(outline) => {
                let b = outline.px_bounds();
                let (w, h) = (b.width() as u32, b.height() as u32);
                let mut alpha = vec![0.0; (w * h) as usize];
                outline.draw(|x, y, c| {
                    if x < w && y < h {
                        alpha[(y * w + x) as usize] = c;
                    }
                });
                // Light text on dark backgrounds reads thin: a gentle boost.
                for a in &mut alpha {
                    *a = a.powf(0.85);
                }
                Cached {
                    rect: self.upload(atlas, w, h, &alpha),
                    left: b.min.x,
                    top: b.min.y,
                }
            }
            None => Cached {
                rect: None,
                left: 0.0,
                top: 0.0,
            },
        };
        if made.rect.is_some() || made.left == 0.0 {
            self.entries.insert(key, made);
        }
        made
    }
    fn corner(&mut self, atlas: &HudAtlas, r: u16, ring: bool) -> Option<Rect> {
        let key = Key::Corner(r, ring);
        if let Some(c) = self.entries.get(&key) {
            return c.rect;
        }
        // The top-left quadrant of a circle centred at (r, r), 4x4 supersampled.
        let n = u32::from(r);
        let rf = f32::from(r);
        let mut alpha = vec![0.0; (n * n) as usize];
        for y in 0..n {
            for x in 0..n {
                let mut hits = 0;
                for sy in 0..4 {
                    for sx in 0..4 {
                        let px = x as f32 + (sx as f32 + 0.5) / 4.0;
                        let py = y as f32 + (sy as f32 + 0.5) / 4.0;
                        let d = ((px - rf).powi(2) + (py - rf).powi(2)).sqrt();
                        if d <= rf && (!ring || d >= rf - 1.0) {
                            hits += 1;
                        }
                    }
                }
                alpha[(y * n + x) as usize] = hits as f32 / 16.0;
            }
        }
        let rect = self.upload(atlas, n, n, &alpha);
        if rect.is_some() {
            self.entries.insert(
                key,
                Cached {
                    rect,
                    left: 0.0,
                    top: 0.0,
                },
            );
        }
        rect
    }
}

/// The launcher's button kinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The accent, for the one thing the screen is for.
    Primary,
    /// Raised with a 1px border.
    Secondary,
    /// Muted text until hovered.
    Ghost,
    /// Ghost in the danger colour (leave, delete).
    Danger,
}
#[derive(Clone, Copy, Debug)]
pub struct Hit {
    pub rect: [f32; 4],
    pub id: u64,
}
/// One frame (or one hit test) of Horizon-style drawing, in dp: the
/// launcher's logical pixels, scaled so the screen reads like the launcher's
/// 1040x700 window at any size.
pub struct Ui<'a> {
    pub atlas: &'a HudAtlas,
    pub batch: &'a mut HudBatch,
    /// The screen in dp.
    pub w: f32,
    pub h: f32,
    /// The cursor in dp.
    pub cursor: (f32, f32),
    /// Physical pixels per dp.
    pub density: f32,
    scale: f32,
    /// Interactive rectangles in drawing order (the last one is on top).
    pub hits: Vec<Hit>,
    real: bool,
    /// Multiplies every colour's alpha (screen fades).
    pub opacity: f32,
    /// Added to every x / y drawn (screen slides).
    pub offset: (f32, f32),
}
impl<'a> Ui<'a> {
    /// `cursor` is in GUI pixels, like every screen's input.
    pub fn new(atlas: &'a HudAtlas, batch: &'a mut HudBatch, cursor: (f32, f32)) -> Self {
        let scale = batch.scale;
        let (pw, ph) = (batch.width * scale, batch.height * scale);
        let density = (pw / 1040.0).min(ph / 700.0).clamp(1.0, 4.0);
        if let Ok(mut w) = WINDOW.lock() {
            *w = [pw.round() as u32, ph.round() as u32];
        }
        with_cache(atlas, |c| {
            let now = Instant::now();
            c.dt = c
                .last_frame
                .map_or(0.0, |t| now.duration_since(t).as_secs_f32().min(0.1));
            c.last_frame = Some(now);
        });
        Self {
            atlas,
            w: pw / density,
            h: ph / density,
            cursor: (cursor.0 * scale / density, cursor.1 * scale / density),
            density,
            scale,
            batch,
            hits: Vec::new(),
            real: true,
            opacity: 1.0,
            offset: (0.0, 0.0),
        }
    }
    /// A layout pass for hit testing: nothing is drawn and no tween starts.
    pub fn dry(atlas: &'a HudAtlas, batch: &'a mut HudBatch, cursor: (f32, f32)) -> Self {
        let mut ui = Self::new(atlas, batch, cursor);
        ui.real = false;
        ui
    }
    /// Move rasterized glyphs into the batch's atlas uploads (real frames only).
    pub fn finish(self) {
        if self.real {
            let pending = with_cache(self.atlas, |c| std::mem::take(&mut c.pending));
            self.batch.patches.extend(pending);
        }
    }
    /// The topmost interactive id under a point in GUI pixels.
    pub fn hit(&self, gui: (f32, f32)) -> Option<u64> {
        let (x, y) = (
            gui.0 * self.scale / self.density,
            gui.1 * self.scale / self.density,
        );
        self.hits
            .iter()
            .rev()
            .find(|h| x >= h.rect[0] && y >= h.rect[1] && x < h.rect[2] && y < h.rect[3])
            .map(|h| h.id)
    }
    /// A tween towards `target` over `secs` with the launcher's
    /// `ease_out_quint`; a new target restarts it from the current value.
    pub fn tween(&self, id: u64, target: f32, secs: f32) -> f32 {
        let real = self.real;
        with_cache(self.atlas, |c| {
            let now = Instant::now();
            match c.anims.get(&id).copied() {
                Some(t) if t.to == target => t.value(),
                Some(t) if real => {
                    let v = t.value();
                    c.anims.insert(
                        id,
                        Tween {
                            from: v,
                            to: target,
                            start: now,
                            secs,
                        },
                    );
                    v
                }
                Some(t) => t.value(),
                None => {
                    if real {
                        c.anims.insert(
                            id,
                            Tween {
                                from: target,
                                to: target,
                                start: now,
                                secs,
                            },
                        );
                    }
                    target
                }
            }
        })
    }
    /// Start a tween from `from` (an entrance).
    pub fn restart(&self, id: u64, from: f32, to: f32, secs: f32) {
        with_cache(self.atlas, |c| {
            c.anims.insert(
                id,
                Tween {
                    from,
                    to,
                    start: Instant::now(),
                    secs,
                },
            );
        });
    }
    pub fn inside(&self, x: f32, y: f32, w: f32, h: f32) -> bool {
        let (cx, cy) = (self.cursor.0 - self.offset.0, self.cursor.1 - self.offset.1);
        cx >= x && cy >= y && cx < x + w && cy < y + h
    }
    /// Register an interactive rectangle; returns its hover layer (0..1,
    /// eased over `QUICK` like the launcher's `hover_layer`).
    pub fn interact(&mut self, id: u64, x: f32, y: f32, w: f32, h: f32, enabled: bool) -> f32 {
        if enabled {
            let (ox, oy) = self.offset;
            self.hits.push(Hit {
                rect: [x + ox, y + oy, x + ox + w, y + oy + h],
                id,
            });
        }
        let over = enabled && self.inside(x, y, w, h);
        self.tween(id, if over { 1.0 } else { 0.0 }, quick())
    }
    /// Whether the pointer is down on this rectangle (a press).
    pub fn pressed(&self, x: f32, y: f32, w: f32, h: f32) -> bool {
        PRESSED.load(Ordering::Relaxed) && self.inside(x, y, w, h)
    }

    // ---- geometry, snapped to physical pixels ----
    fn px_x(&self, v: f32) -> f32 {
        ((v + self.offset.0) * self.density).round()
    }
    fn px_y(&self, v: f32) -> f32 {
        ((v + self.offset.1) * self.density).round()
    }
    fn tint(&self, c: u32) -> u32 {
        if self.opacity >= 1.0 {
            c
        } else {
            color::with_alpha(c, self.opacity)
        }
    }
    fn raw_fill(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, color: u32) {
        if x1 > x0 && y1 > y0 {
            let s = self.scale;
            self.batch.fill(
                self.atlas,
                x0 / s,
                y0 / s,
                (x1 - x0) / s,
                (y1 - y0) / s,
                color,
            );
        }
    }
    pub fn fill(&mut self, x: f32, y: f32, w: f32, h: f32, color: u32) {
        let color = self.tint(color);
        if !self.real || color >> 24 == 0 {
            return;
        }
        let (x0, y0, x1, y1) = (
            self.px_x(x),
            self.px_y(y),
            self.px_x(x + w),
            self.px_y(y + h),
        );
        self.raw_fill(x0, y0, x1, y1, color);
    }
    /// A vertical gradient (top colour to bottom colour).
    pub fn gradient(&mut self, x: f32, y: f32, w: f32, h: f32, top: u32, bottom: u32) {
        self.gradient4(x, y, w, h, [top, top, bottom, bottom]);
    }
    /// A horizontal gradient (left colour to right colour).
    pub fn hgradient(&mut self, x: f32, y: f32, w: f32, h: f32, left: u32, right: u32) {
        self.gradient4(x, y, w, h, [left, right, right, left]);
    }
    /// Corner colours: top-left, top-right, bottom-right, bottom-left.
    pub fn gradient4(&mut self, x: f32, y: f32, w: f32, h: f32, c: [u32; 4]) {
        if !self.real {
            return;
        }
        let c = c.map(|c| self.tint(c));
        let white = self.atlas.sprites["horizon/white"];
        let (cu, cv) = ((white.u0 + white.u1) / 2.0, (white.v0 + white.v1) / 2.0);
        let s = self.scale;
        let (x0, y0) = (self.px_x(x) / s, self.px_y(y) / s);
        let (x1, y1) = (self.px_x(x + w) / s, self.px_y(y + h) / s);
        let p = [[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
        for i in [0, 1, 2, 0, 2, 3] {
            self.batch.vertices.push(crate::hud::HudVertex {
                pos: p[i],
                uv: [cu, cv],
                color: c[i],
            });
        }
    }
    /// The launcher's 1px divider (`border` colour).
    pub fn divider(&mut self, x: f32, y: f32, w: f32) {
        let c = self.tint(color::BORDER);
        if !self.real {
            return;
        }
        let (x0, y0, x1) = (self.px_x(x), self.px_y(y), self.px_x(x + w));
        self.raw_fill(x0, y0, x1, y0 + 1.0, c);
    }
    /// A 1px vertical line.
    pub fn vline(&mut self, x: f32, y: f32, h: f32, color: u32) {
        let c = self.tint(color);
        if !self.real {
            return;
        }
        let (x0, y0, y1) = (self.px_x(x), self.px_y(y), self.px_y(y + h));
        self.raw_fill(x0, y0, x0 + 1.0, y1, c);
    }
    fn corner_quad(&mut self, rect: Rect, x: f32, y: f32, r: f32, flip: (bool, bool), color: u32) {
        let (u0, u1) = if flip.0 {
            (rect.u1, rect.u0)
        } else {
            (rect.u0, rect.u1)
        };
        let (v0, v1) = if flip.1 {
            (rect.v1, rect.v0)
        } else {
            (rect.v0, rect.v1)
        };
        let s = self.scale;
        let (gx, gy, gr) = (x / s, y / s, r / s);
        self.batch.quad(
            [[gx, gy], [gx + gr, gy], [gx + gr, gy + gr], [gx, gy + gr]],
            [[u0, v0], [u1, v0], [u1, v1], [u0, v1]],
            color,
        );
    }
    fn corners(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, r: f32, ring: bool, color: u32) {
        if r < 1.0 {
            return;
        }
        let atlas = self.atlas;
        if let Some(rect) = with_cache(atlas, |c| c.corner(atlas, r as u16, ring)) {
            self.corner_quad(rect, x0, y0, r, (false, false), color);
            self.corner_quad(rect, x1 - r, y0, r, (true, false), color);
            self.corner_quad(rect, x1 - r, y1 - r, r, (true, true), color);
            self.corner_quad(rect, x0, y1 - r, r, (false, true), color);
        }
    }
    /// A filled rectangle with rounded corners.
    pub fn rounded(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32, color: u32) {
        let color = self.tint(color);
        if !self.real || color >> 24 == 0 {
            return;
        }
        let (x0, y0, x1, y1) = (
            self.px_x(x),
            self.px_y(y),
            self.px_x(x + w),
            self.px_y(y + h),
        );
        let r = (radius * self.density)
            .round()
            .min(((x1 - x0) / 2.0).floor())
            .min(((y1 - y0) / 2.0).floor())
            .max(0.0);
        if r < 1.0 {
            self.raw_fill(x0, y0, x1, y1, color);
            return;
        }
        self.raw_fill(x0 + r, y0, x1 - r, y1, color);
        self.raw_fill(x0, y0 + r, x0 + r, y1 - r, color);
        self.raw_fill(x1 - r, y0 + r, x1, y1 - r, color);
        self.corners(x0, y0, x1, y1, r, false, color);
    }
    /// A 1px rounded border.
    pub fn border(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32, color: u32) {
        let color = self.tint(color);
        if !self.real || color >> 24 == 0 {
            return;
        }
        let (x0, y0, x1, y1) = (
            self.px_x(x),
            self.px_y(y),
            self.px_x(x + w),
            self.px_y(y + h),
        );
        let r = (radius * self.density)
            .round()
            .min(((x1 - x0) / 2.0).floor())
            .min(((y1 - y0) / 2.0).floor())
            .max(0.0);
        self.raw_fill(x0 + r, y0, x1 - r, y0 + 1.0, color);
        self.raw_fill(x0 + r, y1 - 1.0, x1 - r, y1, color);
        self.raw_fill(x0, y0 + r, x0 + 1.0, y1 - r, color);
        self.raw_fill(x1 - 1.0, y0 + r, x1, y1 - r, color);
        self.corners(x0, y0, x1, y1, r, true, color);
    }

    // ---- text ----
    /// The advance of `text` in dp.
    pub fn measure(&self, face: Face, size: f32, text: &str) -> f32 {
        let px = (size * self.density * 4.0).round() / 4.0;
        with_cache(self.atlas, |c| {
            let font = c.fonts.face(face);
            let scaled = font.as_scaled(c.fonts.scale(face, px));
            let mut w = 0.0;
            let mut prev = None;
            for ch in text.chars() {
                let id = font.glyph_id(ch);
                if let Some(p) = prev {
                    w += scaled.kern(p, id);
                }
                w += scaled.h_advance(id);
                prev = Some(id);
            }
            w / self.density
        })
    }
    /// Draw a line of text in a `line`-tall box whose top is `y` (the
    /// launcher's line heights: about 1.45 × size for the interface);
    /// returns its width in dp.
    pub fn text_line(
        &mut self,
        face: Face,
        size: f32,
        line: f32,
        x: f32,
        y: f32,
        color: u32,
        text: &str,
    ) -> f32 {
        let color = self.tint(color);
        let px = (size * self.density * 4.0).round() / 4.0;
        let real = self.real && color >> 24 != 0;
        let (x0, y0) = (self.px_x(x), self.px_y(y));
        let box_px = line * self.density;
        let atlas = self.atlas;
        let density = self.density;
        let mut quads = Vec::new();
        let width = with_cache(atlas, |c| {
            let scale = c.fonts.scale(face, px);
            let (ascent, descent) = {
                let f = c.fonts.face(face).as_scaled(scale);
                (f.ascent(), f.descent())
            };
            let baseline = y0 + ((box_px - (ascent - descent)) / 2.0 + ascent).round();
            let mut pen = x0;
            let mut prev = None;
            for ch in text.chars() {
                let (id, advance, kern) = {
                    let font = c.fonts.face(face);
                    let id = font.glyph_id(ch);
                    let f = font.as_scaled(scale);
                    (id, f.h_advance(id), prev.map_or(0.0, |p| f.kern(p, id)))
                };
                pen += kern;
                if real && !ch.is_whitespace() {
                    let g = c.glyph(atlas, face, id, px);
                    if let Some(rect) = g.rect {
                        quads.push(((pen + g.left).round(), baseline + g.top, rect));
                    }
                }
                pen += advance;
                prev = Some(id);
            }
            (pen - x0) / density
        });
        let s = self.scale;
        for (gx, gy, rect) in quads {
            self.batch.rect_uv(
                gx / s,
                gy / s,
                rect.w as f32 / s,
                rect.h as f32 / s,
                rect,
                color,
            );
        }
        width
    }
    /// Text in a `1.45 × size` line box whose top is `y`.
    pub fn text(&mut self, face: Face, size: f32, x: f32, y: f32, color: u32, text: &str) -> f32 {
        self.text_line(face, size, (size * 1.45).round(), x, y, color, text)
    }
    pub fn text_centered(
        &mut self,
        face: Face,
        size: f32,
        cx: f32,
        y: f32,
        color: u32,
        text: &str,
    ) {
        let w = self.measure(face, size, text);
        self.text(face, size, cx - w / 2.0, y, color, text);
    }
    pub fn text_right(
        &mut self,
        face: Face,
        size: f32,
        right: f32,
        y: f32,
        color: u32,
        text: &str,
    ) {
        let w = self.measure(face, size, text);
        self.text(face, size, right - w, y, color, text);
    }
    /// `text` shortened with an ellipsis to fit `max` dp.
    pub fn fit(&self, face: Face, size: f32, text: &str, max: f32) -> String {
        if self.measure(face, size, text) <= max {
            return text.to_owned();
        }
        let mut s: String = text.to_owned();
        while !s.is_empty() && self.measure(face, size, &format!("{s}…")) > max {
            s.pop();
        }
        format!("{}…", s.trim_end())
    }

    // ---- the launcher's components ----
    /// The launcher's `page_header`: an Instrument Serif title and a muted line.
    /// Returns the height used.
    pub fn page_header(&mut self, x: f32, y: f32, title: &str, description: &str) -> f32 {
        self.text_line(Face::Display, TITLE, 32.0, x, y, color::TEXT, title);
        self.text(Face::Regular, BODY, x, y + 38.0, color::MUTED, description);
        38.0 + (BODY * 1.45).round()
    }
    /// The launcher's `button`: 30 tall (or `CONTROL_LARGE` with `LARGE`
    /// text), 6 radius, medium weight, a hover layer easing in over `QUICK`,
    /// and the press state (opacity 0.82 for primary, 0.8 otherwise).
    /// Returns the hover.
    pub fn button(
        &mut self,
        id: u64,
        [x, y, w, h]: [f32; 4],
        label: &str,
        kind: Kind,
        enabled: bool,
    ) -> f32 {
        if self.real {
            // The control socket's button list, in GUI pixels.
            let k = self.density / self.scale;
            let (ox, oy) = self.offset;
            crate::widgets::record(label, [(x + ox) * k, (y + oy) * k, w * k, h * k], enabled);
        }
        let hover = self.interact(id, x, y, w, h, enabled);
        let saved = self.opacity;
        if !enabled {
            self.opacity *= horizon_theme::DISABLED;
        } else if self.pressed(x, y, w, h) {
            self.opacity *= if kind == Kind::Primary {
                horizon_theme::PRESSED
            } else {
                horizon_theme::PRESSED_SOFT
            };
        }
        let fg = match kind {
            Kind::Primary => {
                self.rounded(x, y, w, h, RADIUS, color::ACCENT);
                self.rounded(
                    x,
                    y,
                    w,
                    h,
                    RADIUS,
                    color::with_alpha(color::WHITE, horizon_theme::PRIMARY_HOVER * hover),
                );
                color::ON_ACCENT
            }
            Kind::Secondary => {
                self.rounded(x, y, w, h, RADIUS, color::RAISED);
                self.rounded(x, y, w, h, RADIUS, color::with_alpha(color::HOVER, hover));
                self.border(x, y, w, h, RADIUS, color::BORDER);
                color::TEXT
            }
            Kind::Ghost => {
                self.rounded(x, y, w, h, RADIUS, color::with_alpha(color::HOVER, hover));
                color::mix(color::MUTED, color::TEXT, hover)
            }
            Kind::Danger => {
                self.rounded(x, y, w, h, RADIUS, color::with_alpha(color::HOVER, hover));
                color::DANGER
            }
        };
        let size = if h >= CONTROL_LARGE { LARGE } else { BODY };
        let label = self.fit(Face::Medium, size, label, w - 16.0);
        let lw = self.measure(Face::Medium, size, &label);
        self.text_line(
            Face::Medium,
            size,
            h,
            x + ((w - lw) / 2.0).round(),
            y,
            fg,
            &label,
        );
        self.opacity = saved;
        hover
    }
    /// The width the launcher's button gives a label (padding 12, or 24 large).
    pub fn button_width(&self, label: &str, large: bool) -> f32 {
        let (size, pad) = if large { (LARGE, 24.0) } else { (BODY, 12.0) };
        (self.measure(Face::Medium, size, label) + pad * 2.0).ceil()
    }
    /// The launcher's sidebar list: 30-tall items 2 apart, a raised pill with
    /// a 1px border that slides to the selected item over `MOVE`, and hover
    /// layers on the others. Items are ids `base + index`.
    pub fn sidebar_list(
        &mut self,
        base: u64,
        x: f32,
        y: f32,
        w: f32,
        labels: &[&str],
        selected: usize,
    ) {
        const STEP: f32 = 32.0;
        let pill = self.tween(base ^ 0x9111, selected as f32, move_());
        let py = y + pill * STEP;
        self.rounded(x, py, w, CONTROL, RADIUS, color::RAISED);
        self.border(x, py, w, CONTROL, RADIUS, color::BORDER);
        for (i, label) in labels.iter().enumerate() {
            let iy = y + i as f32 * STEP;
            let active = i == selected;
            let hover = self.interact(base + i as u64, x, iy, w, CONTROL, true);
            if !active {
                self.rounded(
                    x,
                    iy,
                    w,
                    CONTROL,
                    RADIUS,
                    color::with_alpha(color::HOVER, 0.7 * hover),
                );
            }
            let (face, fg) = if active {
                (Face::Medium, color::TEXT)
            } else {
                (Face::Regular, color::MUTED)
            };
            self.text_line(face, BODY, CONTROL, x + 10.0, iy, fg, label);
        }
    }
    /// The launcher's segmented control: a raised frame (radius 7, padding 2,
    /// 1px border) holding 26-tall segments; the active one has the page
    /// background, a border and medium text. Segments are ids `base + index`.
    /// Returns the control's width.
    pub fn segmented(
        &mut self,
        base: u64,
        x: f32,
        y: f32,
        labels: &[&str],
        selected: usize,
    ) -> f32 {
        let widths: Vec<f32> = labels
            .iter()
            .map(|l| (self.measure(Face::Medium, BODY, l) + 28.0).ceil())
            .collect();
        let total = widths.iter().sum::<f32>() + 2.0 * (labels.len() as f32 - 1.0) + 4.0 + 2.0;
        let h = SEGMENT + 6.0;
        self.rounded(x, y, total, h, RADIUS_GROUP, color::RAISED);
        self.border(x, y, total, h, RADIUS_GROUP, color::BORDER);
        let mut sx = x + 3.0;
        for (i, (label, sw)) in labels.iter().zip(&widths).enumerate() {
            let active = i == selected;
            let hover = self.interact(base + i as u64, sx, y + 3.0, *sw, SEGMENT, true);
            if active {
                self.rounded(sx, y + 3.0, *sw, SEGMENT, RADIUS_SEGMENT, color::BG);
                self.border(sx, y + 3.0, *sw, SEGMENT, RADIUS_SEGMENT, color::BORDER);
            }
            let fg = if active {
                color::TEXT
            } else {
                color::mix(color::MUTED, color::TEXT, hover)
            };
            let face = if active { Face::Medium } else { Face::Regular };
            let lw = self.measure(face, BODY, label);
            self.text_line(
                face,
                BODY,
                SEGMENT,
                sx + ((sw - lw) / 2.0).round(),
                y + 3.0,
                fg,
                label,
            );
            sx += sw + 2.0;
        }
        total
    }
    /// The launcher's `switch`: 34x20, accent when on, border colour when off,
    /// a white 16px knob sliding over `QUICK`. Right-aligned at `right`.
    pub fn switch(&mut self, id: u64, right: f32, cy: f32, on: bool) {
        let (w, h) = (34.0, 20.0);
        let (x, y) = (right - w, (cy - h / 2.0).round());
        self.interact(id, x - 4.0, y - 4.0, w + 8.0, h + 8.0, true);
        let t = self.tween(id ^ 0x5717c4, if on { 1.0 } else { 0.0 }, quick());
        self.rounded(
            x,
            y,
            w,
            h,
            h / 2.0,
            color::mix(color::BORDER, color::ACCENT, t),
        );
        self.rounded(x + 2.0 + 14.0 * t, y + 2.0, 16.0, 16.0, 8.0, color::WHITE);
    }
    /// The launcher's settings `row`: a 150-wide muted label column, then the
    /// value; py 12. Returns the row height (draw the action yourself,
    /// right-aligned, and a `divider` under it).
    pub fn row(&mut self, x: f32, y: f32, label: &str, value: &str, value_w: f32) -> f32 {
        let line = (BODY * 1.45).round();
        let h = 12.0 + line.max(CONTROL) + 12.0;
        let ty = y + (h - line) / 2.0;
        self.text(Face::Regular, BODY, x, ty, color::MUTED, label);
        let value = self.fit(Face::Regular, BODY, value, value_w);
        self.text(
            Face::Regular,
            BODY,
            x + ROW_LABEL + 16.0,
            ty,
            color::TEXT,
            &value,
        );
        h
    }
    /// A settings row with, under its label, a one-line description led by
    /// the setting's frame-time cost (`cost` 0 none .. 3 high, coloured from
    /// faint to the danger colour). Same height as [`Ui::row`].
    #[allow(clippy::too_many_arguments)]
    pub fn described_row(
        &mut self,
        x: f32,
        y: f32,
        label: &str,
        value: &str,
        value_w: f32,
        cost: (u8, &str),
        description: &str,
        description_w: f32,
    ) -> f32 {
        let h = 12.0 + CONTROL + 12.0;
        let top = y + 8.0;
        self.text(Face::Regular, BODY, x, top, color::TEXT, label);
        let value = self.fit(Face::Regular, BODY, value, value_w);
        self.text(
            Face::Regular,
            BODY,
            x + ROW_LABEL + 16.0,
            top,
            color::MUTED,
            &value,
        );
        let dy = top + (BODY * 1.45).round();
        let tint = [0.0, 0.2, 0.55, 1.0][usize::from(cost.0.min(3))];
        let cost_color = color::mix(color::FAINT, color::DANGER, tint);
        self.text(Face::Medium, SMALL, x, dy, cost_color, cost.1);
        let used = self.measure(Face::Medium, SMALL, cost.1);
        let rest = self.fit(
            Face::Regular,
            SMALL,
            description,
            (description_w - used - 8.0).max(0.0),
        );
        self.text(
            Face::Regular,
            SMALL,
            x + used + 8.0,
            dy,
            color::FAINT,
            &rest,
        );
        h
    }
    /// A slider in the launcher's language: a 4px track in the border colour
    /// filled with the accent, a white 14px knob. Dragging is the screen's
    /// job (see [`slider_value`]).
    pub fn slider(&mut self, id: u64, x: f32, cy: f32, w: f32, value: f32) {
        let hover = self.interact(id, x - 8.0, cy - 12.0, w + 16.0, 24.0, true);
        let v = value.clamp(0.0, 1.0);
        self.rounded(x, cy - 2.0, w, 4.0, 2.0, color::BORDER);
        self.rounded(x, cy - 2.0, w * v, 4.0, 2.0, color::ACCENT);
        let r = 7.0 + hover;
        self.rounded(x + w * v - r, cy - r, r * 2.0, r * 2.0, r, color::WHITE);
    }
    /// A text field: the launcher's raised control with a 1px border that
    /// takes the accent while focused.
    pub fn field(
        &mut self,
        id: u64,
        [x, y, w, h]: [f32; 4],
        value: &str,
        placeholder: &str,
        focused: bool,
        caret: bool,
    ) {
        let hover = self.interact(id, x, y, w, h, true);
        let focus = self.tween(id ^ 0xf0c05ed, if focused { 1.0 } else { 0.0 }, quick());
        self.rounded(x, y, w, h, RADIUS, color::RAISED);
        self.rounded(
            x,
            y,
            w,
            h,
            RADIUS,
            color::with_alpha(color::HOVER, hover * 0.7),
        );
        self.border(
            x,
            y,
            w,
            h,
            RADIUS,
            color::mix(color::BORDER, color::ACCENT, focus),
        );
        let shown = self.fit(Face::Regular, BODY, value, w - 28.0);
        let tw = if value.is_empty() {
            self.text_line(
                Face::Regular,
                BODY,
                h,
                x + 12.0,
                y,
                color::FAINT,
                placeholder,
            );
            0.0
        } else {
            self.text_line(Face::Regular, BODY, h, x + 12.0, y, color::TEXT, &shown)
        };
        if focused && caret {
            self.fill(x + 12.0 + tw + 1.0, y + 8.0, 1.0, h - 16.0, color::TEXT);
        }
    }
    /// The launcher's atmosphere over the backdrop: a veil of the page
    /// background, darker from the left and toward the bottom.
    pub fn atmosphere(&mut self, veil: f32, shade: f32) {
        let (w, h) = (self.w, self.h);
        self.fill(0.0, 0.0, w, h, color::with_alpha(color::BG, veil));
        self.hgradient(
            0.0,
            0.0,
            w * 0.7,
            h,
            color::with_alpha(color::BG, 0.9 * shade),
            color::with_alpha(color::BG, 0.0),
        );
        self.gradient(
            0.0,
            h * 0.25,
            w,
            h * 0.75,
            color::with_alpha(color::BG, 0.0),
            color::with_alpha(color::BG, 0.75 * shade),
        );
    }
    /// The plain background while the live backdrop loads: the page colour
    /// with a faint cold light from the top.
    pub fn cold_background(&mut self, opacity: f32) {
        let (w, h) = (self.w, self.h);
        let a = |c: u32, k: f32| color::with_alpha(c, k * opacity);
        self.fill(0.0, 0.0, w, h, a(color::BG, 1.0));
        self.gradient(
            0.0,
            0.0,
            w,
            h * 0.7,
            a(color::hex(0x3a5a86, 255), 0.22),
            a(color::hex(0x3a5a86, 255), 0.0),
        );
    }
}
/// The slider value (0..1) for a cursor x in GUI pixels over a track at
/// `x`..`x + w` in dp.
pub fn slider_value(ui: &Ui, x: f32, w: f32, cursor_gui: f32) -> f32 {
    let cx = cursor_gui * ui.scale / ui.density - ui.offset.0;
    ((cx - x) / w).clamp(0.0, 1.0)
}
/// A stable id for a widget from a name and an index.
pub const fn id(name: &str, index: u64) -> u64 {
    // FNV-1a.
    let bytes = name.as_bytes();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut i = 0;
    while i < bytes.len() {
        h ^= bytes[i] as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
        i += 1;
    }
    h ^ index.wrapping_mul(0x9e37_79b9_7f4a_7c15)
}
