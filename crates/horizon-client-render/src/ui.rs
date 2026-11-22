//! Shared GUI widgets drawn as HUD quads: nine-slice sprites (gui.scaling
//! metadata), buttons, sliders, tiled menu backgrounds, separators and the
//! EditBox text field with vanilla cursor, selection and scrolling.
pub mod frame_button;

use crate::{
    hud::{HudAtlas, HudBatch, Rect, rgba},
    text::Text,
};
use horizon_assets::resource_source::ResourceSource;
use std::{collections::HashMap, sync::RwLock, time::Instant};

/// `GuiSpriteScaling.NineSlice`: the sprite's size in GUI pixels, whatever
/// its texture's resolution, its borders (left, top, right, bottom) in the
/// same units, and whether the edges and centre stretch rather than tile.
#[derive(Clone, Copy, Debug, PartialEq)]
struct NineSlice {
    width: u32,
    height: u32,
    border: [u32; 4],
    stretch_inner: bool,
}
/// A GUI sprite's `gui.scaling` metadata.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Scaling {
    Stretch,
    NineSlice(NineSlice),
    Tile { width: u32, height: u32 },
}

/// GUI sprite scaling from `.png.mcmeta` files.
static SCALING: RwLock<Option<HashMap<String, Scaling>>> = RwLock::new(None);

/// A `gui.scaling` section of type `nine_slice`.
fn parse_nine_slice(v: &serde_json::Value) -> Option<NineSlice> {
    let s = &v["gui"]["scaling"];
    if s["type"] != "nine_slice" {
        return None;
    }
    let size = |k: &str| s[k].as_u64().map(|n| n as u32);
    let border = &s["border"];
    let border = if let Some(n) = border.as_u64() {
        [n as u32; 4]
    } else {
        let g = |k: &str| border[k].as_u64().unwrap_or(0) as u32;
        [g("left"), g("top"), g("right"), g("bottom")]
    };
    let (width, height) = (size("width")?, size("height")?);
    // `NineSlice.validate`: borders that meet leave no centre slice, so the
    // sprite has no nine-slice metadata (vanilla draws it as a stretch).
    let [l, t, r, b] = border;
    if l.saturating_add(r) >= width || t.saturating_add(b) >= height {
        return None;
    }
    Some(NineSlice {
        width,
        height,
        border,
        stretch_inner: s["stretch_inner"].as_bool().unwrap_or(false),
    })
}
/// A `gui.scaling` section of type `tile`.
fn parse_tile(v: &serde_json::Value) -> Option<Scaling> {
    let s = &v["gui"]["scaling"];
    if s["type"] != "tile" {
        return None;
    }
    // `GuiSpriteScaling.Tile` reads both through `ExtraCodecs.POSITIVE_INT`
    // (1..=i32::MAX): zero or an out-of-range size leaves the sprite without
    // scaling, so it falls back to a stretch.
    let dimension = |k: &str| {
        i32::try_from(s[k].as_i64()?)
            .ok()
            .filter(|n| *n > 0)
            .map(|n| n as u32)
    };
    Some(Scaling::Tile {
        width: dimension("width")?,
        height: dimension("height")?,
    })
}
/// Parse a `gui.scaling` section, including the explicit stretch form.
fn parse_scaling(v: &serde_json::Value) -> Option<Scaling> {
    let s = &v["gui"]["scaling"];
    match s["type"].as_str()? {
        "stretch" => Some(Scaling::Stretch),
        "nine_slice" => parse_nine_slice(v).map(Scaling::NineSlice),
        "tile" => parse_tile(v),
        _ => None,
    }
}
/// Read `gui.scaling` metadata for every GUI sprite under `root`.
pub fn load_metadata(root: &ResourceSource) {
    set_metadata(read_metadata(root));
}
/// The GUI sprites' `gui.scaling` metadata of a resource root, read off the
/// render thread by a pack reload ([`read_metadata`]) and put in use with
/// [`set_metadata`].
pub struct UiMetadata(HashMap<String, Scaling>);
/// Puts read metadata in use ([`load_metadata`] in two halves).
pub fn set_metadata(metadata: UiMetadata) {
    *SCALING.write().unwrap_or_else(|e| e.into_inner()) = Some(metadata.0);
}
/// Reads the metadata [`load_metadata`] puts in use, without putting it in use. Another
/// namespace's sprites are named with it (`lobby:gui/sprites/...`), as the GUI atlas packs them.
pub fn read_metadata(root: &ResourceSource) -> UiMetadata {
    let mut map = HashMap::new();
    for namespace in root.namespaces("assets") {
        let base = format!("assets/{namespace}/textures/");
        let prefix = match namespace.as_str() {
            "minecraft" => String::new(),
            namespace => format!("{namespace}:"),
        };
        for name in root.list(&format!("{base}gui/sprites/")) {
            let Some(stripped) = name.strip_suffix(".png.mcmeta") else {
                continue;
            };
            let Ok(text) = root.read_to_string(&name) else {
                continue;
            };
            let Some(scaling) = crate::packs::first_json::<serde_json::Value>(text.as_bytes())
                .ok()
                .as_ref()
                .and_then(parse_scaling)
            else {
                continue;
            };
            let key = stripped.strip_prefix(base.as_str()).unwrap_or_default();
            map.insert(format!("{prefix}{key}"), scaling);
        }
    }
    UiMetadata(map)
}
/// The sprite's scaling metadata, when its `.png.mcmeta` has any.
fn scaling_metadata(sprite: &str) -> Option<Scaling> {
    SCALING
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .and_then(|m| m.get(sprite).copied())
}
/// The sprite's scaling. A `nine_slice` caller keeps the existing widget
/// border fallbacks when metadata is absent; `blit_sprite` passes `false` so
/// an unannotated sprite stretches as vanilla does.
fn scaling(sprite: &str, r: &Rect, widget_fallback: bool) -> Scaling {
    if let Some(metadata) = scaling_metadata(sprite) {
        return metadata;
    }
    if !widget_fallback {
        return Scaling::Stretch;
    }
    let border = match sprite {
        "horizon/jei/button_enabled_v2"
        | "horizon/jei/button_disabled_v2"
        | "horizon/jei/button_highlight_v2" => [6; 4],
        "horizon/jei/search_background_v2" => [6; 4],
        "horizon/jei/interactive_ingredient_tooltip_background" => [16; 4],
        "horizon/jei/recipe_preview_background_v2" => [16; 4],
        "horizon/jei/scrollbar_background_v2" => [6; 4],
        "horizon/jei/scrollbar_marker_v2" => [2, 2, 2, 1],
        "gui/sprites/widget/button"
        | "gui/sprites/widget/button_highlighted"
        | "gui/sprites/widget/button_disabled" => [3; 4],
        "gui/sprites/widget/slider_handle" | "gui/sprites/widget/slider_handle_highlighted" => {
            [2, 2, 2, 3]
        }
        _ => [1; 4],
    };
    Scaling::NineSlice(NineSlice {
        width: r.w,
        height: r.h,
        border,
        stretch_inner: false,
    })
}
/// The region `[x, y, w, h]` of a sprite `size` GUI pixels large, in its
/// own units (`TextureAtlasSprite.getU(x / width)`).
fn sub(r: Rect, size: [f32; 2], [x, y, w, h]: [f32; 4]) -> Rect {
    let (du, dv) = ((r.u1 - r.u0) / size[0], (r.v1 - r.v0) / size[1]);
    Rect {
        u0: r.u0 + x * du,
        v0: r.v0 + y * dv,
        u1: r.u0 + (x + w) * du,
        v1: r.v0 + (y + h) * dv,
        w: w as u32,
        h: h as u32,
    }
}
/// `blitTiledSprite`: `src` (a region of a sprite `size` large, in its
/// units) repeated over `dst` from its top left, the last row and column cut.
fn tile(batch: &mut HudBatch, r: Rect, size: [f32; 2], src: [f32; 4], dst: [f32; 4], color: u32) {
    let [sx, sy, sw, sh] = src;
    let [dx, dy, dw, dh] = dst;
    if sw <= 0.0 || sh <= 0.0 || dw <= 0.0 || dh <= 0.0 {
        return;
    }
    let mut y = 0.0;
    while y < dh {
        let h = sh.min(dh - y);
        let mut x = 0.0;
        while x < dw {
            let w = sw.min(dw - x);
            batch.rect_uv(dx + x, dy + y, w, h, sub(r, size, [sx, sy, w, h]), color);
            x += sw;
        }
        y += sh;
    }
}
/// `GuiGraphicsExtractor.blitNineSlicedSprite`: the corners at their size,
/// the edges and centre tiled (or stretched with `stretch_inner`), all in
/// the sprite's `gui.scaling` units, so a high-resolution pack's sprite draws
/// as the default one does.
pub fn nine_slice(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    sprite: &str,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    color: u32,
) {
    if let Some(r) = atlas.sprites.get(sprite) {
        match crate::hud::sprite_modifiers::draws(atlas, sprite, *r, [x, y, w, h], color) {
            Some(draws) => {
                for d in draws {
                    draw_scaling(batch, d.rect, scaling(&d.key, &d.rect, true), d.at, d.color);
                }
            }
            None => draw_scaling(batch, *r, scaling(sprite, r, true), [x, y, w, h], color),
        }
    }
}
/// `GuiGraphicsExtractor.blitSprite`: a sprite over `w` × `h` as its
/// `gui.scaling` says: tiled, nine-sliced, or stretched when it has no
/// metadata.
pub fn blit_sprite(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    sprite: &str,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    color: u32,
) {
    if let Some(r) = atlas.sprites.get(sprite) {
        match crate::hud::sprite_modifiers::draws(atlas, sprite, *r, [x, y, w, h], color) {
            Some(draws) => {
                for d in draws {
                    draw_scaling(
                        batch,
                        d.rect,
                        scaling(&d.key, &d.rect, false),
                        d.at,
                        d.color,
                    );
                }
            }
            None => draw_scaling(batch, *r, scaling(sprite, r, false), [x, y, w, h], color),
        }
    }
}
/// `GuiGraphicsExtractor.blit(location, x, y, u, v, width, height, textureWidth,
/// textureHeight)`: a `src` region of a sprite — in its own texture's units, as the
/// u/v/width/height arguments are — stretched over `w` × `h`.
pub fn blit_sprite_region(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    sprite: &str,
    src: [f32; 4],
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    color: u32,
) {
    if let Some(r) = atlas.sprites.get(sprite) {
        let size = [r.w as f32, r.h as f32];
        batch.rect_uv(x, y, w, h, sub(*r, size, src), color);
    }
}
fn draw_scaling(
    batch: &mut HudBatch,
    r: Rect,
    scaling: Scaling,
    [x, y, w, h]: [f32; 4],
    color: u32,
) {
    match scaling {
        Scaling::Stretch => batch.rect_uv(x, y, w, h, r, color),
        Scaling::NineSlice(n) => draw_nine_slice(batch, r, n, [x, y, w, h], color),
        Scaling::Tile { width, height } => {
            let size = [width as f32, height as f32];
            if size[0] > 0.0 && size[1] > 0.0 {
                tile(
                    batch,
                    r,
                    size,
                    [0.0, 0.0, size[0], size[1]],
                    [x, y, w, h],
                    color,
                );
            }
        }
    }
}
fn draw_nine_slice(
    batch: &mut HudBatch,
    r: Rect,
    n: NineSlice,
    [x, y, w, h]: [f32; 4],
    color: u32,
) {
    let size = [n.width as f32, n.height as f32];
    let [sw, sh] = size;
    let [l, t, rr, b] = n.border.map(|v| v as f32);
    let (half_w, half_h) = ((w / 2.0).floor(), (h / 2.0).floor());
    let (l, rr, t, b) = (l.min(half_w), rr.min(half_w), t.min(half_h), b.min(half_h));
    // blitSprite(sprite, width, height, textureX, textureY, x, y, w, h).
    let part = |batch: &mut HudBatch, tx: f32, ty: f32, dst: [f32; 4]| {
        if dst[2] != 0.0 && dst[3] != 0.0 {
            batch.rect_uv(
                dst[0],
                dst[1],
                dst[2],
                dst[3],
                sub(r, size, [tx, ty, dst[2], dst[3]]),
                color,
            );
        }
    };
    // blitNineSliceInnerSegment.
    let inner = |batch: &mut HudBatch, dst: [f32; 4], src: [f32; 4]| {
        if dst[2] <= 0.0 || dst[3] <= 0.0 {
            return;
        }
        if n.stretch_inner {
            batch.rect_uv(dst[0], dst[1], dst[2], dst[3], sub(r, size, src), color);
        } else {
            tile(batch, r, size, src, dst, color);
        }
    };
    if w == sw && h == sh {
        part(batch, 0.0, 0.0, [x, y, w, h]);
        return;
    }
    if h == sh {
        part(batch, 0.0, 0.0, [x, y, l, h]);
        inner(batch, [x + l, y, w - rr - l, h], [l, 0.0, sw - rr - l, sh]);
        part(batch, sw - rr, 0.0, [x + w - rr, y, rr, h]);
        return;
    }
    if w == sw {
        part(batch, 0.0, 0.0, [x, y, w, t]);
        inner(batch, [x, y + t, w, h - b - t], [0.0, t, sw, sh - b - t]);
        part(batch, 0.0, sh - b, [x, y + h - b, w, b]);
        return;
    }
    let (cw, ch) = (w - rr - l, h - b - t);
    let (scw, sch) = (sw - rr - l, sh - b - t);
    part(batch, 0.0, 0.0, [x, y, l, t]);
    inner(batch, [x + l, y, cw, t], [l, 0.0, scw, t]);
    part(batch, sw - rr, 0.0, [x + w - rr, y, rr, t]);
    part(batch, 0.0, sh - b, [x, y + h - b, l, b]);
    inner(batch, [x + l, y + h - b, cw, b], [l, sh - b, scw, b]);
    part(batch, sw - rr, sh - b, [x + w - rr, y + h - b, rr, b]);
    inner(batch, [x, y + t, l, ch], [0.0, t, l, sch]);
    inner(batch, [x + l, y + t, cw, ch], [l, t, scw, sch]);
    inner(batch, [x + w - rr, y + t, rr, ch], [sw - rr, t, rr, sch]);
}
/// Repeat a whole texture with each copy drawn `size` GUI pixels square.
pub fn tiled(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    sprite: &str,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    size: f32,
    color: u32,
) {
    tiled_sized(batch, atlas, sprite, x, y, w, h, [size, size], color);
}
/// Repeat a texture using its width and height in GUI pixels.
#[allow(clippy::too_many_arguments)]
pub fn tiled_sized(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    sprite: &str,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    size: [f32; 2],
    color: u32,
) {
    let Some(r) = atlas.sprites.get(sprite).copied() else {
        return;
    };
    tile(
        batch,
        r,
        size,
        [0.0, 0.0, size[0], size[1]],
        [x, y, w, h],
        color,
    );
}
/// Text drawn at `scale` times its size (a scaled pose).
pub fn text_scaled(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    text: &Text,
    x: f32,
    y: f32,
    scale: f32,
    color: u32,
) {
    let start = batch.vertices.len();
    batch.text_styled(atlas, text, 0.0, 0.0, color, true);
    for v in &mut batch.vertices[start..] {
        v.pos = [x + v.pos[0] * scale, y + v.pos[1] * scale];
    }
}
pub fn hovered(cursor: (f32, f32), x: f32, y: f32, w: f32, h: f32) -> bool {
    cursor.0 >= x && cursor.0 < x + w && cursor.1 >= y && cursor.1 < y + h
}
/// `TextAlignment.CENTER.calculateLeft` at the box's integer centre:
/// `x + w / 2 - width / 2`. Flooring `(w - width) / 2` sits odd-width labels
/// one pixel left of vanilla when `w` is even.
fn centered_left(x: f32, w: f32, width: f32) -> f32 {
    x + (w / 2.0).floor() - (width / 2.0).floor()
}

/// Centered text with a shadow, for titles and lines of text, which vanilla
/// never scrolls: one wider than `w` keeps the characters that fit. Widget
/// labels go through [`scrolling_text`] instead, as vanilla's do.
pub fn centered_text(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    text: &Text,
    x: f32,
    y: f32,
    w: f32,
    color: u32,
) {
    let text = text.resolved();
    let width = atlas.font.width_text(&text);
    if width <= w {
        batch.text_styled(atlas, &text, centered_left(x, w, width), y, color, true);
    } else {
        // Clip by characters: vanilla clips nothing here, but a title is as
        // wide as the window.
        let mut shown = Text::empty();
        let mut used = 0.0;
        for (i, (c, s)) in text.chars().enumerate() {
            used += atlas.font.advance(c, s);
            if used > w - 2.0 {
                break;
            }
            shown.push(text.slice(i, i + 1));
        }
        batch.text_styled(atlas, &shown, x + 1.0, y, color, true);
    }
}
/// `ActiveTextCollector.defaultScrollingHelper`'s slide: a label `overflow`
/// pixels too wide moves from its end back to its start and out again, one
/// sine period every `max(overflow * 0.5, 3)` seconds of `Util.getMillis`.
fn scroll_offset(overflow: f32, now_ms: i64) -> f64 {
    let period = (f64::from(overflow) * 0.5).max(3.0);
    let seconds = now_ms as f64 / 1000.0;
    let phase = (std::f64::consts::TAU * seconds / period).cos();
    let pos = (std::f64::consts::FRAC_PI_2 * phase).sin() / 2.0 + 0.5;
    f64::from(overflow) * pos
}

/// Where an overflowing label's left edge is drawn: `left - (int) pos`.
fn scrolled_left(left: f32, overflow: f32, now_ms: i64) -> f32 {
    left - scroll_offset(overflow, now_ms) as i32 as f32
}

/// A widget's label as `AbstractWidget.extractScrollingStringOverContents`
/// draws it, at `now_ms`: centred between `left` and `right` when it fits,
/// else drawn from the left and slid (`defaultScrollingHelper`) inside the
/// widget's box. `y` is the label's top line.
#[allow(clippy::too_many_arguments)]
pub fn scrolling_text_at(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    text: &Text,
    left: f32,
    right: f32,
    top: f32,
    bottom: f32,
    y: f32,
    now_ms: i64,
    color: u32,
) {
    // `acceptScrollingWithDefaultCenter`: `centerX = (left + right) / 2`.
    let center_x = left + ((right - left) / 2.0).floor();
    scrolling_text_around_at(
        batch, atlas, text, center_x, left, right, top, bottom, y, now_ms, color,
    );
}

/// [`scrolling_text_at`] around an explicit `center_x`
/// (`ActiveTextCollector.acceptScrolling`, which `SpriteIconButton.TextAndIcon`
/// calls with the button's middle): a label that fits is centred on `center_x`
/// moved as far as it must to stay between `left` and `right`
/// (`Mth.clamp(centerX, left + lineWidth / 2, right - lineWidth / 2)`), so
/// beside an icon it ends where the icon's room begins. A wider one is drawn
/// from the left and slid inside the box.
#[allow(clippy::too_many_arguments)]
pub fn scrolling_text_around_at(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    text: &Text,
    center_x: f32,
    left: f32,
    right: f32,
    top: f32,
    bottom: f32,
    y: f32,
    now_ms: i64,
    color: u32,
) {
    let text = text.resolved();
    let width = atlas.font.width_text(&text);
    let room = right - left;
    if width <= room {
        let half = (width / 2.0).floor();
        let text_x = center_x.max(left + half).min(right - half);
        batch.text_styled(atlas, &text, text_x - half, y, color, true);
        return;
    }
    // Scissored to the widget: what does not fit is read as the label
    // slides, never dropped.
    let start = batch.vertices.len();
    batch.text_styled(
        atlas,
        &text,
        scrolled_left(left, width - room, now_ms),
        y,
        color,
        true,
    );
    batch.clip_since(start, left, top, right, bottom);
}

/// [`scrolling_text_at`] at `Util.getMillis`.
#[allow(clippy::too_many_arguments)]
pub fn scrolling_text(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    text: &Text,
    left: f32,
    right: f32,
    top: f32,
    bottom: f32,
    y: f32,
    color: u32,
) {
    scrolling_text_at(
        batch,
        atlas,
        text,
        left,
        right,
        top,
        bottom,
        y,
        crate::hud_overlays::now_ms(),
        color,
    );
}

/// `MenuTabBar.MenuTabButton.renderLabel` for the tab at `x, y`: its label
/// inside the tab less 1 px on each side, from the tab's top when selected
/// and 3 px lower when not (`acceptScrollingWithDefaultCenter`, so one wider
/// than the tab slides instead of running into the next tab). White, gray
/// for an inactive tab (`WithInactiveMessage`).
pub fn menu_tab_label(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    label: &Text,
    (x, y, width, height): (f32, f32, f32, f32),
    selected: bool,
    active: bool,
    now_ms: i64,
) {
    let top = y + if selected { 0.0 } else { 3.0 };
    let bottom = y + height;
    scrolling_text_at(
        batch,
        atlas,
        label,
        x + 1.0,
        x + width - 1.0,
        top,
        bottom,
        // defaultScrollingHelper: `(top + bottom - lineHeight) / 2 + 1`.
        ((top + bottom - 9.0) / 2.0).floor() + 1.0,
        now_ms,
        if active { u32::MAX } else { 0xFFA0_A0A0 },
    );
}

/// Button (AbstractButton): nine-slice sprite plus the centred label.
pub fn button(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    label: &Text,
    active: bool,
    hover: bool,
) {
    button_as(batch, atlas, x, y, w, h, label, active, hover, |rect| {
        crate::widgets::Narratable::button(label.plain(), rect, active)
    });
}
/// A [`button`] with a job on its screen: its controller button and glyph come from `action`, so
/// a new screen gets them by drawing this for its Done, Back, Cancel or Apply.
#[allow(clippy::too_many_arguments)]
pub fn action_button(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    action: crate::widgets::Action,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    label: &Text,
    active: bool,
    hover: bool,
) {
    crate::widgets::declare_action(action, [x, y, w, h], active);
    let room = if active {
        crate::widgets::action_glyph_room()
    } else {
        0.0
    };
    button_with_room(
        batch,
        atlas,
        [x, y, w, h],
        label,
        active,
        hover,
        false,
        room,
        |rect| crate::widgets::Narratable::button(label.plain(), rect, active),
    );
}
/// A [`button`] that vanilla narrates as another widget (a `CycleButton`, a
/// `LockIconButton`): `narration` builds what it records from its rectangle,
/// and only while the narrator records.
#[allow(clippy::too_many_arguments)]
pub fn button_as(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    label: &Text,
    active: bool,
    hover: bool,
    narration: impl FnOnce([f32; 4]) -> crate::widgets::Narratable,
) {
    button_with_room(
        batch,
        atlas,
        [x, y, w, h],
        label,
        active,
        hover,
        false,
        0.0,
        narration,
    );
}
/// An options button whose focus highlights it without changing the mouse cursor.
/// `AbstractWidget.handleCursor` tests hover, not `isHoveredOrFocused`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn focused_button(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    rect: [f32; 4],
    label: &Text,
    active: bool,
    hover: bool,
    focused: bool,
    action: Option<crate::widgets::Action>,
) {
    let room = if let Some(action) = action {
        crate::widgets::declare_action(action, rect, active);
        if active {
            crate::widgets::action_glyph_room()
        } else {
            0.0
        }
    } else {
        0.0
    };
    button_with_room(
        batch,
        atlas,
        rect,
        label,
        active,
        hover,
        focused,
        room,
        |rect| crate::widgets::Narratable::button(label.plain(), rect, active),
    );
}
/// A button whose label keeps `room` free at its left.
#[allow(clippy::too_many_arguments)]
fn button_with_room(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    [x, y, w, h]: [f32; 4],
    label: &Text,
    active: bool,
    hover: bool,
    focused: bool,
    room: f32,
    narration: impl FnOnce([f32; 4]) -> crate::widgets::Narratable,
) {
    let plain = label.plain();
    crate::widgets::record(&plain, [x, y, w, h], active);
    // An icon button has no label to record, and A presses it all the same.
    if active && plain.trim().is_empty() {
        crate::widgets::target([x, y, w, h]);
    }
    crate::widgets::narratable(|| narration([x, y, w, h]).hovered(hover || focused));
    crate::widgets::widget_cursor(hover, active);
    let sprite = if !active {
        "gui/sprites/widget/button_disabled"
    } else if hover || focused {
        "gui/sprites/widget/button_highlighted"
    } else {
        "gui/sprites/widget/button"
    };
    nine_slice(batch, atlas, sprite, x, y, w, h, u32::MAX);
    let color = if active {
        rgba(255, 255, 255, 255)
    } else {
        rgba(160, 160, 160, 255)
    };
    // Centred on the button, as the title's are, and only moved out from under a glyph.
    scrolling_text_around_at(
        batch,
        atlas,
        label,
        x + (w / 2.0).floor(),
        x + 2.0 + room,
        x + w - 2.0,
        y,
        y + h,
        y + ((h - 8.0) / 2.0).floor(),
        crate::hud_overlays::now_ms(),
        color,
    );
}
/// AbstractSliderButton: track, 8px handle at `value` (0..1) and the label.
pub fn slider(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    x: f32,
    y: f32,
    w: f32,
    label: &Text,
    value: f32,
    hover: bool,
    active: bool,
    dragging: bool,
) {
    slider_ext(
        batch, atlas, x, y, w, label, value, hover, false, false, active, dragging,
    );
}

/// `AbstractSliderButton.extractWidgetRenderState`: track, 8px handle at `value` (0..1),
/// with support for keyboard focus and `canChangeValue`.
pub fn slider_ext(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    x: f32,
    y: f32,
    w: f32,
    label: &Text,
    value: f32,
    hover: bool,
    focused: bool,
    can_change_value: bool,
    active: bool,
    dragging: bool,
) {
    let kind = crate::widgets::Kind::Slider {
        value,
        track: [x + 4.0, w - 8.0],
    };
    // The control socket's widget tree (`ui`): its label and value.
    crate::widgets::record_as(&label.plain(), [x, y, w, 20.0], active, kind);
    crate::widgets::narratable(|| {
        crate::widgets::Narratable::slider(label.plain(), [x, y, w, 20.0], active).hovered(hover)
    });
    if active {
        // A controller's D-pad can focus it (Horizon's controller support).
        crate::widgets::target_as([x, y, w, 20.0], kind);
    }
    let (track, handle) = slider_sprites_ext(hover, active, focused, can_change_value, dragging);
    nine_slice(batch, atlas, track, x, y, w, 20.0, u32::MAX);
    nine_slice(
        batch,
        atlas,
        handle,
        x + (value.clamp(0.0, 1.0) * (w - 8.0)).floor(),
        y,
        8.0,
        20.0,
        u32::MAX,
    );
    let color = if active {
        u32::MAX
    } else {
        rgba(160, 160, 160, 255)
    };
    scrolling_text(
        batch,
        atlas,
        label,
        x + 2.0,
        x + w - 2.0,
        y,
        y + 20.0,
        y + 6.0,
        color,
    );
}
#[cfg(test)]
fn slider_sprites(hover: bool, active: bool, dragging: bool) -> (&'static str, &'static str) {
    slider_sprites_ext(hover, active, false, false, dragging)
}

/// `AbstractSliderButton.getSprite` and `getHandleSprite`: the track outline
/// lights while the slider is focused and its value cannot change
/// (`isActive() && isFocused() && !canChangeValue`), the handle while it is
/// hovered or its value can change (`isActive() && (isHovered || canChangeValue)`).
/// `canChangeValue` is set when the slider is focused with the mouse or Tab and
/// flipped by Enter (`setFocused`, `keyPressed`): Horizon's `can_change_value`
/// for the keyboard, and a drag for the mouse, which focuses it with the value
/// free to change. So a dragged slider keeps its plain track.
fn slider_sprites_ext(
    hover: bool,
    active: bool,
    focused: bool,
    can_change_value: bool,
    dragging: bool,
) -> (&'static str, &'static str) {
    let can_change_value = can_change_value || dragging;
    let track = if active && focused && !can_change_value {
        "gui/sprites/widget/slider_highlighted"
    } else {
        "gui/sprites/widget/slider"
    };
    let handle = if active && (hover || can_change_value) {
        "gui/sprites/widget/slider_handle_highlighted"
    } else {
        "gui/sprites/widget/slider_handle"
    };
    (track, handle)
}
/// Slider value for a click at `cx` (AbstractSliderButton.setValueFromMouse).
pub fn slider_value(x: f32, w: f32, cx: f32) -> f32 {
    ((cx - (x + 4.0)) / (w - 8.0)).clamp(0.0, 1.0)
}
/// Screen.renderMenuBackground: in a world the translucent dark texture,
/// otherwise the dirt-like menu texture, tiled at 32 pixels.
#[horizon_mod_api_macros::replaceable]
pub fn menu_background(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    in_world: bool,
) {
    let sprite = if in_world {
        "gui/inworld_menu_background"
    } else {
        "gui/menu_background"
    };
    if atlas.sprites.contains_key(sprite) {
        tiled(batch, atlas, sprite, x, y, w, h, 32.0, u32::MAX);
    } else {
        batch.fill(atlas, x, y, w, h, rgba(0, 0, 0, 160));
    }
}
/// Header and footer separators of list screens.
pub fn separators(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    header_bottom: f32,
    footer_top: f32,
    in_world: bool,
) {
    let (header, footer) = if in_world {
        (
            "gui/inworld_header_separator",
            "gui/inworld_footer_separator",
        )
    } else {
        ("gui/header_separator", "gui/footer_separator")
    };
    let w = batch.width;
    tiled_sized(
        batch,
        atlas,
        header,
        0.0,
        header_bottom - 2.0,
        w,
        2.0,
        [32.0, 2.0],
        u32::MAX,
    );
    tiled_sized(
        batch,
        atlas,
        footer,
        0.0,
        footer_top,
        w,
        2.0,
        [32.0, 2.0],
        u32::MAX,
    );
}
/// List background between the separators.
pub fn list_background(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    top: f32,
    bottom: f32,
    in_world: bool,
) {
    let sprite = if in_world {
        "gui/inworld_menu_list_background"
    } else {
        "gui/menu_list_background"
    };
    tiled(
        batch,
        atlas,
        sprite,
        0.0,
        top,
        batch.width,
        bottom - top,
        32.0,
        u32::MAX,
    );
}
/// `AbstractScrollArea.scrollerHeight`: `Mth.clamp((height * height) / content, 32, height - 8)`.
/// Java applies the minimum then the maximum, so a list shorter than 40 pixels
/// keeps a thumb of its last 8 pixels instead of panicking as `f32::clamp` does.
fn scroller_height(height: f32, content: f32) -> f32 {
    (height * height / content)
        .trunc()
        .max(32.0)
        .min(height - 8.0)
}
/// The visible scrollbar thumb, shared by painting and pointer input.
pub(crate) fn scrollbar_thumb(
    top: f32,
    bottom: f32,
    content: f32,
    scroll: f32,
) -> Option<(f32, f32)> {
    let height = bottom - top;
    if content <= height {
        return None;
    }
    let bar = scroller_height(height, content).floor();
    let y = top + ((scroll / (content - height)) * (height - bar)).floor();
    Some((y, bar))
}

/// `AbstractScrollArea`'s scrolling of a list `top..bottom` holding `content`
/// pixels: the amount, the mouse wheel and the held scrollbar.
#[derive(Clone, Copy, Debug, Default)]
pub struct Scroll {
    pub amount: f32,
    /// The pointer's last y while the scrollbar is held (`scrolling`).
    held: Option<f32>,
}
impl Scroll {
    fn max(top: f32, bottom: f32, content: f32) -> f32 {
        (content - (bottom - top)).max(0.0)
    }
    /// `setScrollAmount`: kept within the content.
    pub fn clamp(&mut self, top: f32, bottom: f32, content: f32) {
        self.amount = self.amount.clamp(0.0, Self::max(top, bottom, content));
    }
    /// `mouseScrolled`: `rate` pixels per wheel step.
    pub fn wheel(&mut self, delta: f32, rate: f32, top: f32, bottom: f32, content: f32) {
        self.amount -= delta * rate;
        self.clamp(top, bottom, content);
    }
    /// `updateScrolling`: a press on the scrollbar column at `bar_x` (when
    /// there is something to scroll) holds it.
    pub fn press(
        &mut self,
        (x, y): (f32, f32),
        bar_x: f32,
        top: f32,
        bottom: f32,
        content: f32,
    ) -> bool {
        let over = x >= bar_x && x <= bar_x + 6.0 && y >= top && y < bottom;
        self.held = (over && Self::max(top, bottom, content) > 0.0).then_some(y);
        self.held.is_some()
    }
    pub fn held(&self) -> bool {
        self.held.is_some()
    }
    /// `mouseDragged` while held: above or below the list jumps to an end,
    /// otherwise the thumb follows the pointer.
    pub fn drag(&mut self, y: f32, top: f32, bottom: f32, content: f32) {
        let Some(last) = self.held else { return };
        let max = Self::max(top, bottom, content);
        if y < top {
            self.amount = 0.0;
        } else if y > bottom {
            self.amount = max;
        } else {
            let height = bottom - top;
            let bar = scroller_height(height, content);
            self.amount += (y - last) * (max.max(1.0) / (height - bar)).max(1.0);
        }
        self.clamp(top, bottom, content);
        self.held = Some(y);
    }
    /// `onRelease`.
    pub fn release(&mut self) {
        self.held = None;
    }
}

/// Vertical scroll bar of a list (AbstractScrollArea): the sprites, and the
/// list is registered for the controller's focus (`widgets::scroll_area`).
pub fn scrollbar(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    x: f32,
    top: f32,
    bottom: f32,
    content: f32,
    scroll: f32,
) {
    if content > bottom - top {
        crate::widgets::scroll_area(
            [0.0, top, x + 6.0, bottom - top],
            scroll,
            content - (bottom - top),
        );
    }
    scrollbar_sprites(batch, atlas, x, top, bottom, content, scroll);
}

/// The scroll bar's background and scroller sprites.
pub fn scrollbar_sprites(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    x: f32,
    top: f32,
    bottom: f32,
    content: f32,
    scroll: f32,
) {
    let Some((y, bar)) = scrollbar_thumb(top, bottom, content, scroll) else {
        return;
    };
    let height = bottom - top;
    nine_slice(
        batch,
        atlas,
        "gui/sprites/widget/scroller_background",
        x,
        top,
        6.0,
        height,
        u32::MAX,
    );
    nine_slice(
        batch,
        atlas,
        "gui/sprites/widget/scroller",
        x,
        y,
        6.0,
        bar,
        u32::MAX,
    );
}

/// Screen tooltip (DefaultTooltipPositioner + TooltipRenderUtil): the
/// background and frame sprites around the lines, beside the cursor.
pub fn tooltip(batch: &mut HudBatch, atlas: &HudAtlas, lines: &[Text], cursor: (f32, f32)) {
    if lines.is_empty() {
        return;
    }
    let (w, h) = tooltip_size(atlas, lines);
    let mut x = cursor.0 + 12.0;
    let mut y = cursor.1 - 12.0;
    if x + w > batch.width {
        x = (cursor.0 - 24.0 - w).max(4.0);
    }
    if y + h + 3.0 > batch.height {
        y = batch.height - h - 3.0;
    }
    tooltip_box(batch, atlas, lines, x, y, w, h);
}
/// `WidgetTooltipHolder.createTooltipPositioner` for a hovered widget
/// (`MenuTooltipPositioner`): the tooltip follows the pointer's x and keeps
/// 3 to 5 px from the widget at `rect`.
pub fn tooltip_menu(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    lines: &[Text],
    cursor: (f32, f32),
    rect: [f32; 4],
) {
    if lines.is_empty() {
        return;
    }
    let (w, h) = tooltip_size(atlas, lines);
    let (x, y) = menu_tooltip_position(
        (batch.width as i32, batch.height as i32),
        (cursor.0 as i32, cursor.1 as i32),
        rect.map(|v| v as i32),
        (w.ceil() as i32, h.ceil() as i32),
    );
    tooltip_box(batch, atlas, lines, x as f32, y as f32, w, h);
}
/// `BelowOrAboveWidgetTooltipPositioner.positionTooltip`: a keyboard-focused widget's tooltip.
pub fn tooltip_focused(batch: &mut HudBatch, atlas: &HudAtlas, lines: &[Text], rect: [f32; 4]) {
    if lines.is_empty() {
        return;
    }
    let (w, h) = tooltip_size(atlas, lines);
    let (x, y) = focused_tooltip_position(
        (batch.width as i32, batch.height as i32),
        rect.map(|v| v as i32),
        (w.ceil() as i32, h.ceil() as i32),
    );
    tooltip_box(batch, atlas, lines, x as f32, y as f32, w, h);
}

/// `BelowOrAboveWidgetTooltipPositioner.positionTooltip`: below, or above when it does not fit.
fn focused_tooltip_position(screen: (i32, i32), rect: [i32; 4], size: (i32, i32)) -> (i32, i32) {
    let [left, top, width, height] = rect;
    let mut x = left + 3;
    let mut y = top + height + 4;
    if y + size.1 + 3 > screen.1 {
        y = top - size.1 - 4;
    }
    if x + size.0 > screen.0 {
        x = (left + width - size.0 - 3).max(4);
    }
    (x, y)
}

/// A widget's tooltip while the controller's focus is on it (Horizon's: vanilla has no gamepad
/// input, and the hidden pointer mid-widget would put it over the widget).
pub fn tooltip_under(batch: &mut HudBatch, atlas: &HudAtlas, lines: &[Text], rect: [f32; 4]) {
    if lines.is_empty() {
        return;
    }
    let (w, h) = tooltip_size(atlas, lines);
    let (x, y) = tooltip_under_position((batch.width, batch.height), rect, (w, h));
    tooltip_box(batch, atlas, lines, x, y, w, h);
}
/// Where `tooltip_under` puts lines of `size`: the frame's top left a pixel under the widget's
/// bottom left, over the widget when the screen ends first, and always on the screen.
fn tooltip_under_position(screen: (f32, f32), rect: [f32; 4], size: (f32, f32)) -> (f32, f32) {
    // The frame reaches this far past the lines on every side.
    const FRAME: f32 = 4.0;
    let [x, y, _, height] = rect;
    let right = screen.0 - size.0 - FRAME;
    let below = y + height + 1.0 + FRAME;
    let top = if below + size.1 + FRAME <= screen.1 {
        below
    } else {
        y - 1.0 - FRAME - size.1
    };
    ((x + FRAME).min(right).max(FRAME), top.max(FRAME))
}
/// `MenuTooltipPositioner.positionTooltip`: the tooltip's top left with the
/// pointer at `cursor`, the widget it belongs to at `rect`, and the screen.
/// `MARGIN` 5 and `MOUSE_OFFSET_X` 12 on the horizontal, and the
/// `MAX_OVERLAP_WITH_WIDGET` 3 and `MAX_DISTANCE_TO_WIDGET` 5 of the vertical
/// offset from the widget's top or bottom edge.
pub fn menu_tooltip_position(
    screen: (i32, i32),
    cursor: (i32, i32),
    rect: [i32; 4],
    size: (i32, i32),
) -> (i32, i32) {
    let [_, y, _, height] = rect;
    let height = height.max(1);
    let mut pos = (cursor.0 + 12, cursor.1);
    if pos.0 + size.0 > screen.0 - 5 {
        pos.0 = (cursor.0 - 12 - size.0).max(9);
    }
    pos.1 += 3;
    // `bottom() + 3 + getOffset(0, 0, height)` is the widget's edge, `height`
    // further down; below that the tooltip goes under it, else over it.
    let below = size.1 + 6;
    if y + 2 * height + below <= screen.1 - 5 {
        pos.1 += widget_offset(pos.1, y, height);
    } else {
        pos.1 -= below + widget_offset(pos.1, y + height, height);
    }
    pos
}
/// `MenuTooltipPositioner.getOffset`: how far the tooltip goes past the
/// widget's top (or bottom) edge, `height - 3` (`MAX_OVERLAP_WITH_WIDGET`)
/// with the pointer on the edge, down to `MAX_DISTANCE_TO_WIDGET` 5 a whole
/// height away. The distance over the height is a float division (`i2f`,
/// `fdiv` in 26.3's bytecode), so the offset moves smoothly with the pointer;
/// then `Mth.lerp` and Java's float rounding.
fn widget_offset(at: i32, edge: i32, height: i32) -> i32 {
    let distance = (at - edge).unsigned_abs().min(height as u32) as f32;
    let start = (height - 3) as f32;
    horizon_core::jmath::round_f32(start + distance / height as f32 * (5.0 - start))
}
fn tooltip_size(atlas: &HudAtlas, lines: &[Text]) -> (f32, f32) {
    let w = lines
        .iter()
        .map(|l| atlas.font.width_text(l))
        .fold(0.0, f32::max);
    let h = if lines.len() == 1 {
        8.0
    } else {
        10.0 * lines.len() as f32
    };
    (w, h)
}
/// `TooltipRenderUtil.getBackgroundSprite` and `getFrameSprite` of a tooltip style
/// (`<namespace>:<path>`): `tooltip/<path>_background` and `tooltip/<path>_frame` in the style's
/// namespace, named as the GUI atlas packs them.
pub fn tooltip_style_sprites(namespace: &str, path: &str) -> [String; 2] {
    let prefix = match namespace {
        "minecraft" => String::new(),
        namespace => format!("{namespace}:"),
    };
    ["background", "frame"].map(|part| format!("{prefix}gui/sprites/tooltip/{path}_{part}"))
}
/// `TooltipRenderUtil.renderTooltipBackground`: the background sprite, then the frame sprite,
/// each drawn by `blitSprite` (nine-sliced as its `gui.scaling` says, stretched without it)
/// 12 pixels out from the tooltip's contents `[x, y, w, h]` (`PADDING` 3 and `MARGIN` 9 on
/// every side). Whether the atlas has both sprites; one that lacks either draws nothing.
pub fn tooltip_sprites(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    [background, frame]: [&str; 2],
    [x, y, w, h]: [f32; 4],
) -> bool {
    if ![background, frame]
        .iter()
        .all(|sprite| atlas.sprites.contains_key(*sprite))
    {
        return false;
    }
    for sprite in [background, frame] {
        blit_sprite(
            batch,
            atlas,
            sprite,
            x - 12.0,
            y - 12.0,
            w + 24.0,
            h + 24.0,
            u32::MAX,
        );
    }
    true
}
/// Vanilla's tooltip panel round content at `rect` (`TooltipRenderUtil`): its background and
/// frame sprites, from the resource packs, reaching 12 GUI pixels out. For anything drawn as a
/// tooltip that isn't lines of text beside the cursor.
pub fn tooltip_panel(batch: &mut HudBatch, atlas: &HudAtlas, rect: [f32; 4]) {
    tooltip_sprites(
        batch,
        atlas,
        [
            "gui/sprites/tooltip/background",
            "gui/sprites/tooltip/frame",
        ],
        rect,
    );
}
fn tooltip_box(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    lines: &[Text],
    x: f32,
    y: f32,
    w: f32,
    h: f32,
) {
    crate::widgets::frame_tooltip(lines);
    tooltip_panel(batch, atlas, [x, y, w, h]);
    let mut ly = y;
    for (i, line) in lines.iter().enumerate() {
        batch.text_styled(atlas, line, x, ly, u32::MAX, true);
        ly += if i == 0 { 12.0 } else { 10.0 };
    }
}

/// The EditBox text field.
#[derive(Clone, Debug)]
pub struct EditBox {
    /// The text, cursor, selection anchor, maximum length and whether line breaks are kept (a
    /// book page's `MultiLineEditBox`): the editing core mods' text fields share. Its fields
    /// read and write through the box (`edit_box.value`).
    pub edit: mod_api::client::widgets::TextEdit,
    display: usize,
    pub bordered: bool,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub focused: bool,
    focused_at: Instant,
    /// Grey completion shown after the text (EditBox.setSuggestion).
    pub suggestion: Option<String>,
    pub text_color: u32,
    /// `EditBox.setCentered`: the text starts in the middle of the box's
    /// width, not at its left edge (`BookSignScreen`'s title).
    pub centered: bool,
    /// `EditBox.textColorUneditable`: the colour of a box `setEditable(false)`
    /// made read-only, which still shows its value and caret.
    pub text_color_uneditable: u32,
    /// `EditBox.isEditable`: a read-only box keeps the focus, the caret keys
    /// and copy, but takes no typed text, Backspace/Delete or paste (`AnvilScreen`
    /// sets it by its input slot).
    pub editable: bool,
    /// `getMessage`: what the narrator calls the box ("<message> edit box:
    /// <value>").
    pub message: String,
    /// `EditBox.setInvertHighlightedTextColor`: a selection draws its text
    /// inverted (the default) or in the box's own colour (`AnvilScreen`).
    pub invert_highlighted_text_color: bool,
    /// Where a borderless box drawn in a frame of its own focuses: a controller's outline goes
    /// round the frame, not over the text.
    pub frame: Option<[f32; 4]>,
}
/// The focused text field of a screen that keeps its values as `String`s
/// (the world list's search, world names, seeds, and the server screens): the `EditBox` state that
/// outlives a frame, its cursor, selection and scroll.
#[derive(Clone, Debug)]
pub struct Caret {
    field: Option<crate::clipboard::FieldId>,
    edit: EditBox,
    /// The last click's time and field, for double clicks.
    last_click: Option<(Instant, crate::clipboard::FieldId)>,
    /// The button is held since a press on the field (`onDrag`).
    held: bool,
}
impl Default for Caret {
    fn default() -> Self {
        Self {
            field: None,
            edit: EditBox::new(0.0, 0.0, 200.0, 20.0),
            last_click: None,
            held: false,
        }
    }
}
impl Caret {
    /// The box for field `id` holding `value`. A field newly focused, or
    /// whose value changed elsewhere, starts with its cursor at the end.
    fn sync(&mut self, id: crate::clipboard::FieldId, value: &str, max: usize) -> &mut EditBox {
        self.edit.max_length = max;
        if self.field != Some(id) || self.edit.value != value {
            self.edit.value = value.to_owned();
            self.edit.move_cursor_to_end(false);
            if self.field != Some(id) {
                self.edit.display = 0;
                self.field = Some(id);
            }
        }
        &mut self.edit
    }
    /// Edit field `id`'s `value` through its box; the value is written back.
    pub fn edit<R>(
        &mut self,
        id: crate::clipboard::FieldId,
        value: &mut String,
        max: usize,
        f: impl FnOnce(&mut EditBox) -> R,
    ) -> R {
        let edit = self.sync(id, value, max);
        let result = f(edit);
        value.clone_from(&edit.value);
        result
    }
    /// A press on field `id` at GUI `x` (`EditBox.onClick`): a second press
    /// within 250 ms is a double click (`MouseHandler`).
    pub fn press(
        &mut self,
        atlas: &HudAtlas,
        (id, value, max): (crate::clipboard::FieldId, &mut String, usize),
        rect: [f32; 4],
        x: f32,
        shift: bool,
    ) {
        let now = Instant::now();
        let double = self
            .last_click
            .is_some_and(|(at, field)| field == id && now.duration_since(at).as_millis() < 250);
        self.last_click = Some((now, id));
        self.held = true;
        self.edit(id, value, max, |edit| {
            [edit.x, edit.y, edit.width, edit.height] = rect;
            edit.click(atlas, x, shift, double);
        });
    }
    /// The pointer moved with the button held after a press on field `id`.
    pub fn drag(
        &mut self,
        atlas: &HudAtlas,
        (id, value, max): (crate::clipboard::FieldId, &mut String, usize),
        x: f32,
    ) {
        if self.held && self.last_click.is_some_and(|(_, field)| field == id) {
            self.edit(id, value, max, |edit| edit.drag_to(atlas, x));
        }
    }
    /// The button was released: drags no longer select.
    pub fn release(&mut self) {
        self.held = false;
    }
    /// Draw field `id` at `rect`: the focused one with its caret, the others
    /// showing the end of their text.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        (id, value, max): (crate::clipboard::FieldId, &str, usize),
        rect: [f32; 4],
        focused: bool,
        focused_at: Instant,
    ) {
        self.draw_color(
            batch,
            atlas,
            (id, value, max),
            rect,
            focused,
            focused_at,
            rgba(224, 224, 224, 255),
        );
    }
    /// [`Self::draw`] with the text in `color` (`EditBox.setTextColor`): the
    /// settings tab turns an invalid Realm name red.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_color(
        &mut self,
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        (id, value, max): (crate::clipboard::FieldId, &str, usize),
        rect: [f32; 4],
        focused: bool,
        focused_at: Instant,
        color: u32,
    ) {
        let mut idle;
        let edit = if focused {
            self.sync(id, value, max)
        } else {
            idle = EditBox::new(0.0, 0.0, 0.0, 0.0);
            idle.max_length = max;
            idle.set_value(value);
            &mut idle
        };
        [edit.x, edit.y, edit.width, edit.height] = rect;
        edit.focused = focused;
        edit.text_color = color;
        if edit.focused_at < focused_at {
            edit.focused_at = focused_at;
        }
        edit.draw(batch, atlas, None);
    }
}

/// `EditBox.extractWidgetRenderState` for Horizon's append-only fields,
/// unbordered and with the cursor at the end: the text from (x, y), the
/// highlight when Select All covers the value (`invert_text` as
/// `setInvertHighlightedTextColor`), and the cursor, blinking from when the
/// field took the focus (`TextCursorUtils.isCursorVisible`): an underscore,
/// or a bar once the value is `max_chars` long. `shadow` is `setTextShadow`.
#[allow(clippy::too_many_arguments)]
pub fn append_field(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    value: &str,
    (x, y, width): (f32, f32, f32),
    (color, shadow): (u32, bool),
    max_chars: usize,
    selected: bool,
    invert_text: bool,
    focused_ms: u128,
) {
    let mut cursor_x = x;
    if !value.is_empty() {
        batch.text(atlas, value, x, y, color, shadow);
        cursor_x += HudBatch::text_width(atlas, value) + 1.0;
    }
    let insert = utf16_len(value) >= max_chars;
    if insert {
        cursor_x -= 1.0;
    }
    if selected && !value.is_empty() {
        let right = x + width;
        let (x0, x1) = (cursor_x.min(right), (x - 1.0).min(right));
        batch.text_highlight(atlas, (x1, y - 1.0, x0 - x1, 11.0), invert_text);
    }
    if (focused_ms / 300).is_multiple_of(2) {
        if insert {
            batch.fill(atlas, cursor_x, y - 1.0, 1.0, 11.0, color);
        } else {
            batch.text(atlas, "_", cursor_x, y, color, shadow);
        }
    }
}
// The editing core's helpers, shared with mods' text fields (`mod_api::client::widgets`).
use mod_api::client::widgets::byte_index;
pub use mod_api::client::widgets::{
    char_len, java_whitespace, multiline_word_position, utf16_len, utf16_prefix, word_position,
};
/// `EditBox.deleteWords(dir)` for an append-only field, whose caret is at the
/// end: removes the word in that direction. True when the value changed.
pub fn delete_words(value: &mut String, dir: isize) -> bool {
    let len = char_len(value);
    let to = word_position(value, dir, len);
    if to == len {
        return false;
    }
    value.truncate(byte_index(value, to));
    true
}
impl crate::clipboard::Editable for EditBox {
    fn select_all(&mut self) {
        self.move_cursor_to_end(false);
        self.highlight = 0;
    }
    fn selected_text(&self) -> String {
        self.edit.selected_text()
    }
    fn insert(&mut self, text: &str) -> bool {
        let before = self.value.clone();
        self.insert_text(text);
        self.value != before
    }
}
impl std::ops::Deref for EditBox {
    type Target = mod_api::client::widgets::TextEdit;
    fn deref(&self) -> &Self::Target {
        &self.edit
    }
}
impl std::ops::DerefMut for EditBox {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.edit
    }
}
impl EditBox {
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            edit: mod_api::client::widgets::TextEdit::default(),
            display: 0,
            bordered: true,
            x,
            y,
            width,
            height,
            focused: true,
            focused_at: Instant::now(),
            suggestion: None,
            text_color: rgba(224, 224, 224, 255),
            centered: false,
            text_color_uneditable: rgba(112, 112, 112, 255),
            editable: true,
            message: String::new(),
            invert_highlighted_text_color: true,
            frame: None,
        }
    }
    pub fn inner_width(&self) -> f32 {
        if self.bordered {
            self.width - 8.0
        } else {
            self.width
        }
    }
    pub fn text_x(&self) -> f32 {
        if self.bordered { self.x + 4.0 } else { self.x }
    }
    pub fn text_y(&self) -> f32 {
        if self.bordered {
            self.y + ((self.height - 8.0) / 2.0).floor()
        } else {
            self.y
        }
    }
    /// `EditBox.updateTextPosition`: where the text starts, `(width - text
    /// width) / 2` in for a centred box (vanilla's `textX`), else at
    /// `text_x` (4 in from a border).
    pub fn text_start_x(&self, text_width: f32) -> f32 {
        if self.centered {
            self.x + ((self.width - text_width) / 2.0).floor()
        } else {
            self.text_x()
        }
    }
    pub fn reset_blink(&mut self) {
        self.focused_at = Instant::now();
    }
    /// The caret is showing (`EditBox`'s blink, on for 300 ms of every 600).
    pub fn cursor_visible(&self) -> bool {
        self.focused && (self.focused_at.elapsed().as_millis() / 300).is_multiple_of(2)
    }
    /// After the core moved the cursor once: scroll to it and restart the blink, as
    /// `EditBox.moveCursorTo` does.
    fn moved(&mut self) {
        self.scroll_to(self.edit.cursor);
        self.reset_blink();
    }
    pub fn set_value(&mut self, value: &str) {
        self.edit.set_value(value);
        self.moved();
    }
    /// EditBox.insertText: replace the selection, respecting the maximum
    /// length. The limit is in UTF-16 code units, as Java's `String.length()`
    /// counts them.
    pub fn insert_text(&mut self, text: &str) {
        if !self.editable {
            return;
        }
        self.edit.insert_text(text);
        self.moved();
    }
    /// Delete `n` characters (negative: backwards) or the selection. A
    /// read-only box consumes the key without changing its value.
    pub fn delete_chars(&mut self, n: isize) {
        if !self.editable {
            return;
        }
        self.edit.delete_chars(n);
        self.moved();
    }
    pub fn set_cursor(&mut self, pos: usize, select: bool) {
        self.edit.set_cursor(pos, select);
        self.moved();
    }
    pub fn move_cursor_to_end(&mut self, select: bool) {
        self.edit.move_cursor_to_end(select);
        self.moved();
    }
    /// EditBox.findClickedPositionInText: the character boundary under GUI `x`.
    pub fn position_at(&mut self, atlas: &HudAtlas, x: f32) -> usize {
        self.layout(atlas);
        let rest: String = self.value.chars().skip(self.display).collect();
        let shown: String = rest
            .chars()
            .take(atlas.font.fit(&rest, self.inner_width()))
            .collect();
        let start = self.text_start_x(atlas.font.width_text(&Text::literal(shown.as_str())));
        let room = (x.floor() - start).min(self.inner_width());
        self.display + atlas.font.fit(&rest, room)
    }
    /// EditBox.onClick: the cursor to the pointer, extending the selection
    /// with Shift; a double click selects the word under it (`selectWord`).
    pub fn click(&mut self, atlas: &HudAtlas, x: f32, shift: bool, double: bool) {
        let pos = self.position_at(atlas, x);
        if double {
            let (start, end) = self.edit.word_at(pos);
            self.set_cursor(start, false);
            self.set_cursor(end, true);
        } else {
            self.set_cursor(pos, shift);
        }
    }
    /// `MultilineTextField.selectWordAtCursor` on a book page: the word before
    /// the cursor is selected, its end found past non-whitespace only
    /// (`getWordEndPosition`).
    pub fn select_word_at_cursor(&mut self) {
        let begin = multiline_word_position(&self.value, -1, self.cursor);
        let end = begin
            + self
                .value
                .chars()
                .skip(begin)
                .take_while(|c| !java_whitespace(*c))
                .count();
        self.set_cursor(begin, false);
        self.set_cursor(end, true);
    }
    /// EditBox.onDrag: the selection follows the pointer.
    pub fn drag_to(&mut self, atlas: &HudAtlas, x: f32) {
        let pos = self.position_at(atlas, x);
        self.set_cursor(pos, true);
    }
    fn scroll_to(&mut self, pos: usize) {
        // EditBox.scrollTo without font access is completed in `layout`.
        self.display = self.display.min(char_len(&self.value));
        if pos == self.display && pos > 0 {
            // A cursor on the first shown character steps back about a field's width
            // (`displayPos -= plainSubstrByWidth(value, width, true).length()`): `layout` scrolls
            // forward from the start until the cursor is at the right edge.
            self.display = 0;
        } else if pos < self.display {
            self.display = pos;
        }
    }
    fn layout(&mut self, atlas: &HudAtlas) {
        let len = char_len(&self.value);
        self.display = self.display.min(len);
        let (value, inner) = (self.value.clone(), self.inner_width());
        let visible = |d: usize| -> usize {
            let rest: String = value.chars().skip(d).collect();
            atlas.font.fit(&rest, inner)
        };
        while self.cursor > self.display + visible(self.display) && self.display < len {
            self.display += 1;
        }
        if self.cursor < self.display {
            self.display = self.cursor;
        }
    }
    /// Keyboard editing shared by every text field. Returns true if handled.
    pub fn key(&mut self, key: winit::keyboard::KeyCode, ctrl: bool, shift: bool) -> bool {
        use mod_api::client::widgets::EditKey;
        use winit::keyboard::KeyCode as K;
        let key = match key {
            K::Backspace => EditKey::Backspace,
            K::Delete => EditKey::Delete,
            K::ArrowLeft => EditKey::Left,
            K::ArrowRight => EditKey::Right,
            K::Home => EditKey::Home,
            K::End => EditKey::End,
            K::KeyA => EditKey::SelectAll,
            _ => return false,
        };
        if !self.editable && matches!(key, EditKey::Backspace | EditKey::Delete) {
            return true;
        }
        let handled = self.edit.key(key, ctrl, shift);
        if handled {
            self.moved();
        }
        handled
    }
    /// Ctrl+A/C/V/X (`EditBox.keyPressed`). True when the value changed.
    pub fn shortcut(
        &mut self,
        shortcut: crate::clipboard::EditShortcut,
        clipboard: &mut dyn crate::clipboard::Clipboard,
    ) -> bool {
        use crate::clipboard::EditShortcut;
        if !self.editable {
            match shortcut {
                // EditBox.keyPressed: Cut copies the selection, then does not
                // take it out of a read-only box.
                EditShortcut::Cut => {
                    clipboard.set(&self.selected_text());
                    return false;
                }
                EditShortcut::Paste => return false,
                EditShortcut::SelectAll | EditShortcut::Copy => {}
            }
        }
        let before = self.value.clone();
        crate::clipboard::apply(self, shortcut, clipboard);
        self.reset_blink();
        self.value != before
    }
    /// The first shown character after scrolling to the cursor, and the
    /// milliseconds since the field was focused (the cursor's blink).
    pub fn view_state(&mut self, atlas: &HudAtlas) -> (usize, u64) {
        self.layout(atlas);
        (self.display, self.focused_at.elapsed().as_millis() as u64)
    }
    /// Draw the field; `format` styles the visible text (command colouring).
    pub fn draw(&mut self, batch: &mut HudBatch, atlas: &HudAtlas, format: Option<&Text>) {
        let caret = self.cursor_visible();
        self.draw_with_caret(batch, atlas, format, caret);
    }
    /// The first character shown (`EditBox.displayPos`): a field kept outside the box (a
    /// mod's) hands back the scroll its last draw answered.
    pub fn set_scroll(&mut self, first: usize) {
        self.display = first.min(char_len(&self.value));
    }
    /// After a draw: the first character shown and the GUI x of each shown character
    /// boundary, from the start of the shown text to its end.
    pub fn shown_edges(&self, atlas: &HudAtlas) -> (usize, Vec<f32>) {
        let rest: String = self.value.chars().skip(self.display).collect();
        let shown = atlas.font.fit(&rest, self.inner_width());
        let edges = (0..=shown)
            .map(|n| self.text_x() + atlas.font.width(&rest.chars().take(n).collect::<String>()))
            .collect();
        (self.display, edges)
    }
    /// [`Self::draw`] with the caret's blink decided by the caller: `caret` shows it (when it
    /// is in view) for a field whose blink a mod keeps.
    pub fn draw_with_caret(
        &mut self,
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        format: Option<&Text>,
        caret: bool,
    ) {
        self.draw_skinned_caret(batch, atlas, format, caret, None);
    }
    /// Draw with a custom nine-slice background and the field's own caret blink.
    pub fn draw_with_skin(
        &mut self,
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        format: Option<&Text>,
        skin: Option<&str>,
    ) {
        let caret = self.cursor_visible();
        self.draw_skinned_caret(batch, atlas, format, caret, skin);
    }
    fn draw_skinned_caret(
        &mut self,
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        format: Option<&Text>,
        caret: bool,
        skin: Option<&str>,
    ) {
        self.layout(atlas);
        // The focus outline and a press go round the box's frame where it has one.
        let rect = self
            .frame
            .unwrap_or([self.x, self.y, self.width, self.height]);
        crate::widgets::target_as(rect, crate::widgets::Kind::Field);
        crate::widgets::narratable(|| {
            crate::widgets::Narratable::edit_box(self.message.clone(), self.value.clone(), rect)
                .focused(self.focused)
        });
        if self.bordered {
            let sprite = skin.unwrap_or(if self.focused {
                "gui/sprites/widget/text_field_highlighted"
            } else {
                "gui/sprites/widget/text_field"
            });
            nine_slice(
                batch,
                atlas,
                sprite,
                self.x,
                self.y,
                self.width,
                self.height,
                u32::MAX,
            );
        }
        let color = if self.editable {
            self.text_color
        } else {
            self.text_color_uneditable
        };
        let rest: String = self.value.chars().skip(self.display).collect();
        let shown_len = atlas.font.fit(&rest, self.inner_width());
        let displayed: String = rest.chars().take(shown_len).collect();
        let rel_cursor = self.cursor as isize - self.display as isize;
        let on_screen = rel_cursor >= 0 && rel_cursor as usize <= shown_len;
        let show_cursor = caret && on_screen;
        let styled = |from: usize, to: usize| -> Text {
            match format {
                Some(t) => t.slice(self.display + from, self.display + to),
                None => Text::literal(
                    displayed
                        .chars()
                        .skip(from)
                        .take(to - from)
                        .collect::<String>(),
                ),
            }
        };
        let start = if self.centered {
            self.text_start_x(atlas.font.width_text(&Text::literal(displayed.as_str())))
        } else {
            self.text_x()
        };
        let (tx, ty) = (start, self.text_y());
        let mut draw_x = tx;
        if !displayed.is_empty() {
            let half = if on_screen {
                rel_cursor as usize
            } else {
                shown_len
            };
            let t = styled(0, half);
            batch.text_styled(atlas, &t, draw_x, ty, color, true);
            draw_x += atlas.font.width_text(&t) + 1.0;
        }
        // EditBox.extractWidgetRenderState: `value.length() >= maxLength`
        // is Java's length, so the caret turns into a bar at the unit count.
        let insert =
            self.cursor < char_len(&self.value) || utf16_len(&self.value) >= self.max_length;
        let mut cursor_x = draw_x;
        if !on_screen {
            cursor_x = if rel_cursor > 0 { tx + self.width } else { tx };
        } else if insert {
            cursor_x -= 1.0;
            draw_x -= 1.0;
        }
        if !displayed.is_empty() && on_screen && (rel_cursor as usize) < shown_len {
            batch.text_styled(
                atlas,
                &styled(rel_cursor as usize, shown_len),
                draw_x,
                ty,
                color,
                true,
            );
        }
        if !insert && let Some(s) = &self.suggestion {
            batch.text(atlas, s, cursor_x - 1.0, ty, rgba(128, 128, 128, 255), true);
        }
        let rel_highlight =
            (self.highlight as isize - self.display as isize).clamp(0, shown_len as isize) as usize;
        if rel_highlight as isize != rel_cursor {
            let hx = tx
                + atlas
                    .font
                    .width(&displayed.chars().take(rel_highlight).collect::<String>());
            // textHighlight between the cursor and the anchor, clipped at the right edge.
            let right = self.x + self.width;
            let (x0, x1) = (cursor_x.min(right), (hx - 1.0).min(right));
            let (a, b) = (x0.min(x1), x0.max(x1));
            batch.text_highlight(
                atlas,
                (a, ty - 1.0, b - a, 11.0),
                self.invert_highlighted_text_color,
            );
        }
        if show_cursor {
            if insert {
                batch.fill(atlas, cursor_x, ty - 1.0, 1.0, 11.0, color);
            } else {
                batch.text(atlas, "_", cursor_x, ty, color, true);
            }
        }
        if self.focused {
            crate::preedit::report_caret(cursor_x, ty, 10.0);
        }
    }
    /// Character index under GUI x (mouse click).
    pub fn index_at(&self, atlas: &HudAtlas, x: f32) -> usize {
        let rest: String = self.value.chars().skip(self.display).collect();
        self.display + atlas.font.fit(&rest, (x - self.text_x()).max(0.0))
    }
    /// GUI x of a character index (EditBox.getScreenX).
    pub fn screen_x(&self, atlas: &HudAtlas, index: usize) -> f32 {
        if index > char_len(&self.value) {
            return self.x;
        }
        let shown: String = self
            .value
            .chars()
            .skip(self.display)
            .take(index.saturating_sub(self.display))
            .collect();
        self.text_x() + atlas.font.width(&shown)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn notes_after_a_sprites_metadata_keep_its_nine_slice() {
        // Gson stops reading after the first value, so vanilla keeps this nine slice
        // (CozyUI+'s highlighted button, #3046).
        let root = std::env::temp_dir().join(format!("horizon-ui-meta-{}", std::process::id()));
        let widget = root.join("assets/minecraft/textures/gui/sprites/widget");
        std::fs::create_dir_all(&widget).unwrap();
        std::fs::write(
            widget.join("button_highlighted.png.mcmeta"),
            "{\"gui\": {\"scaling\": {\"type\": \"nine_slice\", \"width\": 80, \"height\": 20,\n\
             \"border\": {\"left\": 34, \"top\": 8, \"right\": 34, \"bottom\": 8}}}}\n\n// 0  10\n// 1  8\n",
        )
        .unwrap();
        let metadata = read_metadata(&ResourceSource::directory(root.clone()));
        std::fs::remove_dir_all(&root).unwrap();
        let Some(Scaling::NineSlice(slice)) =
            metadata.0.get("gui/sprites/widget/button_highlighted")
        else {
            panic!("the button keeps its nine-slice metadata");
        };
        assert_eq!(
            (slice.width, slice.height, slice.border),
            (80, 20, [34, 8, 34, 8])
        );
    }

    /// The quads' destination rectangles and their UVs.
    fn quads(batch: &HudBatch) -> Vec<([f32; 4], [f32; 4])> {
        batch
            .vertices
            .chunks(6)
            .map(|q| {
                let (a, c) = (q[0], q[2]);
                (
                    [a.pos[0], a.pos[1], c.pos[0], c.pos[1]],
                    [a.uv[0], a.uv[1], c.uv[0], c.uv[1]],
                )
            })
            .collect()
    }
    /// Destination and UV rectangles equal to within f32 rounding.
    fn assert_quad(actual: ([f32; 4], [f32; 4]), expected: ([f32; 4], [f32; 4])) {
        let close = |a: [f32; 4], b: [f32; 4]| a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-5);
        assert!(
            close(actual.0, expected.0) && close(actual.1, expected.1),
            "{actual:?} != {expected:?}"
        );
    }
    /// A sprite filling the atlas, `w x h` texels as a 32x pack draws it.
    fn sprite(w: u32, h: u32) -> Rect {
        Rect {
            u0: 0.0,
            v0: 0.0,
            u1: 1.0,
            v1: 1.0,
            w,
            h,
        }
    }
    fn faithful(json: &str) -> NineSlice {
        parse_nine_slice(&serde_json::from_str(json).unwrap()).unwrap()
    }
    #[test]
    fn centered_label_matches_vanilla_integer_center() {
        // Even box, odd text: (w - width) / 2 floors one pixel left of w/2 - width/2.
        assert_eq!(centered_left(0.0, 200.0, 9.0), 96.0);
        assert_eq!(centered_left(0.0, 200.0, 8.0), 96.0);
        assert_eq!(centered_left(10.0, 196.0, 9.0), 104.0);
        assert_eq!(centered_left(2.0, 196.0, 7.0), 97.0);
    }
    /// `defaultScrollingHelper`: one period every `max(overflow * 0.5, 3)`
    /// seconds, a full sine: the label starts with its end showing, shows its
    /// start half a period later, and never slides past either end.
    #[test]
    fn an_overflowing_label_slides_from_its_end_to_its_start_and_back() {
        let overflow = 40.0;
        // max(40 * 0.5, 3) = 20 s.
        assert!((scroll_offset(overflow, 0) - 40.0).abs() < 1.0e-9);
        assert!(scroll_offset(overflow, 10_000).abs() < 1.0e-9);
        assert!((scroll_offset(overflow, 20_000) - 40.0).abs() < 1.0e-9);
        // A label one pixel too wide waits the floor of three seconds.
        assert!(scroll_offset(1.0, 1_500).abs() < 1.0e-9);
        assert!((scroll_offset(1.0, 0) - 1.0).abs() < 1.0e-9);
        for ms in 0..40_000 {
            let offset = scroll_offset(overflow, ms);
            assert!((0.0..=40.0).contains(&offset), "{ms} ms: {offset}");
        }
    }

    fn atlas() -> HudAtlas {
        HudAtlas::build(&horizon_assets::resource_source::ResourceSource::directory(
            (std::path::Path::new("/nonexistent/horizon-test-assets")).to_owned(),
        ))
        .unwrap()
    }

    /// `defaultScrollingHelper` draws an overflowing label from `left` less the
    /// integer part of the slide, so it shows its end first and its start half
    /// a period later, and it moves a whole pixel at a time.
    #[test]
    fn an_overflowing_label_is_drawn_left_less_the_integer_slide() {
        // max(40 * 0.5, 3) = 20 s: at 0 s the whole overflow is slid, at 10 s none.
        assert_eq!(scrolled_left(10.0, 40.0, 0), -30.0);
        assert_eq!(scrolled_left(10.0, 40.0, 10_000), 10.0);
        // A fractional slide is cut, not rounded.
        let offset = scroll_offset(40.0, 1_000) as i32 as f32;
        assert!(offset > 0.0);
        assert_eq!(scrolled_left(10.0, 40.0, 1_000), 10.0 - offset);
    }

    /// A controller reaches an icon button only through its target, and can't press a disabled one.
    #[test]
    fn an_active_icon_button_is_a_focus_target_and_a_disabled_one_is_not() {
        let atlas = atlas();
        let mut batch = HudBatch::scratch([640, 480]);
        // Whatever this thread drew before is the last frame's, then gone.
        crate::widgets::end_frame(u64::MAX - 1);
        button(
            &mut batch,
            &atlas,
            10.0,
            20.0,
            20.0,
            20.0,
            &Text::empty(),
            true,
            false,
        );
        button(
            &mut batch,
            &atlas,
            40.0,
            20.0,
            20.0,
            20.0,
            &Text::empty(),
            false,
            false,
        );
        crate::widgets::end_frame(u64::MAX);
        let targets: Vec<[f32; 4]> = crate::widgets::last_targets()
            .iter()
            .map(|t| t.rect)
            .collect();
        assert_eq!(targets, [[10.0, 20.0, 20.0, 20.0]]);
    }

    #[test]
    fn a_glyph_leaves_a_label_that_fits_centred() {
        let atlas = atlas();
        crate::widgets::set_action_glyphs(true);
        let label = Text::literal("Done");
        let mut plain = HudBatch::scratch([640, 480]);
        button(
            &mut plain, &atlas, 10.0, 20.0, 150.0, 20.0, &label, true, false,
        );
        let mut declared = HudBatch::scratch([640, 480]);
        action_button(
            &mut declared,
            &atlas,
            crate::widgets::Action::Done,
            10.0,
            20.0,
            150.0,
            20.0,
            &label,
            true,
            false,
        );
        assert_eq!(quads(&plain), quads(&declared));
    }

    #[test]
    fn a_label_wider_than_its_widget_stays_inside_it() {
        let atlas = atlas();
        let label = Text::literal("Graphics API: Prefer Vulkan (Experimental)");
        assert!(atlas.font.width_text(&label.resolved()) > 96.0);
        let mut batch = HudBatch::scratch([640, 480]);
        scrolling_text_at(
            &mut batch,
            &atlas,
            &label,
            10.0,
            106.0,
            20.0,
            40.0,
            26.0,
            0,
            u32::MAX,
        );
        assert!(!batch.vertices.is_empty());
        for v in &batch.vertices {
            assert!(
                (10.0..=106.0).contains(&v.pos[0]) && (20.0..=40.0).contains(&v.pos[1]),
                "{:?} outside the widget",
                v.pos
            );
        }
        // A label that fits is centred and not cut: every glyph is drawn.
        let mut batch = HudBatch::scratch([640, 480]);
        let short = Text::literal("Done");
        scrolling_text_at(
            &mut batch,
            &atlas,
            &short,
            10.0,
            106.0,
            20.0,
            40.0,
            26.0,
            0,
            u32::MAX,
        );
        let width = atlas.font.width_text(&short.resolved());
        let left = centered_left(10.0, 96.0, width);
        let lo = batch
            .vertices
            .iter()
            .map(|v| v.pos[0])
            .fold(f32::INFINITY, f32::min);
        let hi = batch
            .vertices
            .iter()
            .map(|v| v.pos[0])
            .fold(f32::NEG_INFINITY, f32::max);
        assert!(
            (lo - left).abs() < 0.001,
            "centred at {left}, drawn from {lo}"
        );
        assert!(
            hi - lo >= width - 0.001,
            "the whole label is drawn: {lo}..{hi}, {width} wide"
        );
    }

    /// `MenuTabButton.renderLabel` on a 24 px tab: centred on the tab, its
    /// text top at `(0 + 24 - 9) / 2 + 1 = 8` when selected and at
    /// `(3 + 24 - 9) / 2 + 1 = 10` when not; white on an active tab, selected
    /// or not, `0xA0A0A0` on an inactive one (`WithInactiveMessage`). The
    /// Statistics tabs drew every label at 8 and an active unselected one gray.
    #[test]
    fn a_tab_label_sits_lower_on_an_unselected_tab_and_is_gray_only_when_inactive() {
        let atlas = atlas();
        let label = Text::literal("Items");
        let half = (atlas.font.width_text(&label.resolved()) / 2.0).floor();
        // A 124 px tab at x = 100: `(left + right) / 2 = (101 + 223) / 2 = 162`.
        let left = 162.0 - half;
        for (selected, active, y, color) in [
            (true, true, 8.0, u32::MAX),
            (false, true, 10.0, u32::MAX),
            (false, false, 10.0, 0xFFA0_A0A0),
        ] {
            let mut tab = HudBatch::scratch([640, 480]);
            menu_tab_label(
                &mut tab,
                &atlas,
                &label,
                (100.0, 0.0, 124.0, 24.0),
                selected,
                active,
                0,
            );
            let mut want = HudBatch::scratch([640, 480]);
            want.text_styled(&atlas, &label.resolved(), left, y, color, true);
            let glyphs = |b: &HudBatch| -> Vec<([u32; 2], u32)> {
                b.vertices
                    .iter()
                    .filter(|v| v.text == 1)
                    .map(|v| ([v.pos[0].to_bits(), v.pos[1].to_bits()], v.color))
                    .collect()
            };
            assert!(!glyphs(&want).is_empty());
            assert_eq!(
                glyphs(&tab),
                glyphs(&want),
                "selected {selected}, active {active}"
            );
        }
    }

    /// `AbstractSliderButton.getSprite`/`getHandleSprite`: a focused slider
    /// whose value can't change lights its track, a hovered one or one whose
    /// value can change (Tab or Enter, or the mouse's drag) lights its handle.
    #[test]
    fn slider_sprites_follow_focus_and_the_drag() {
        // Hovered, at rest: plain track, lit handle.
        let (track, handle) = slider_sprites(true, true, false);
        assert_eq!(track, "gui/sprites/widget/slider");
        assert_eq!(handle, "gui/sprites/widget/slider_handle_highlighted");
        // Dragged: still a plain track, lit handle.
        let (track, handle) = slider_sprites(true, true, true);
        assert_eq!(track, "gui/sprites/widget/slider");
        assert_eq!(handle, "gui/sprites/widget/slider_handle_highlighted");
        // Focused with the arrow keys: the value can't change, the track lights.
        let (track, handle) = slider_sprites_ext(false, true, true, false, false);
        assert_eq!(track, "gui/sprites/widget/slider_highlighted");
        assert_eq!(handle, "gui/sprites/widget/slider_handle");
        // Focused with Tab, or Enter pressed on it: the value can change, the handle lights.
        let (track, handle) = slider_sprites_ext(false, true, true, true, false);
        assert_eq!(track, "gui/sprites/widget/slider");
        assert_eq!(handle, "gui/sprites/widget/slider_handle_highlighted");
        // Focused while the mouse drags: the track stays plain.
        let (track, _) = slider_sprites_ext(true, true, true, false, true);
        assert_eq!(track, "gui/sprites/widget/slider");
        // Not hovered, not dragged: nothing is lit.
        let (track, handle) = slider_sprites(false, true, false);
        assert_eq!(track, "gui/sprites/widget/slider");
        assert_eq!(handle, "gui/sprites/widget/slider_handle");
        // An inactive slider highlights neither, even when focused.
        let (track, handle) = slider_sprites_ext(true, false, true, true, true);
        assert_eq!(track, "gui/sprites/widget/slider");
        assert_eq!(handle, "gui/sprites/widget/slider_handle");
    }
    #[test]
    fn separator_tiles_sample_both_rows_and_clip_the_last_column() {
        let mut batch = HudBatch::scratch([640, 480]);
        tile(
            &mut batch,
            sprite(32, 2),
            [32.0, 2.0],
            [0.0, 0.0, 32.0, 2.0],
            [0.0, 5.0, 40.0, 2.0],
            u32::MAX,
        );
        let q = quads(&batch);
        assert_eq!(q.len(), 2);
        assert_quad(q[0], ([0.0, 5.0, 32.0, 7.0], [0.0, 0.0, 1.0, 1.0]));
        assert_quad(q[1], ([32.0, 5.0, 40.0, 7.0], [0.0, 0.0, 0.25, 1.0]));
    }
    #[test]
    fn a_border_that_leaves_no_centre_slice_is_no_metadata() {
        // NineSlice.validate rejects 12 + 12 >= 20, so the sprite has none.
        let json = serde_json::json!({"gui": {"scaling": {
            "type": "nine_slice", "width": 20, "height": 20, "border": 12}}});
        assert!(parse_nine_slice(&json).is_none());
        // 9 + 9 < 20 still leaves a centre slice.
        let json = serde_json::json!({"gui": {"scaling": {
            "type": "nine_slice", "width": 20, "height": 20, "border": 9}}});
        assert_eq!(parse_nine_slice(&json).unwrap().border, [9; 4]);
    }
    #[test]
    fn a_high_resolution_button_slices_in_its_gui_units() {
        // Faithful 32x's widget/button: 400x40 texels, gui.scaling 200x20.
        let n = faithful(
            r#"{"gui":{"scaling":{"type":"nine_slice","width":200,"height":20,
                "border":{"left":20,"top":4,"right":20,"bottom":4}}}}"#,
        );
        let mut batch = HudBatch::scratch([640, 480]);
        draw_nine_slice(
            &mut batch,
            sprite(400, 40),
            n,
            [10.0, 5.0, 150.0, 20.0],
            u32::MAX,
        );
        let q = quads(&batch);
        // Full height: left border, the centre tiled, right border
        // (blitNineSlicedSprite's height == nineSlice.height case).
        assert_quad(q[0], ([10.0, 5.0, 30.0, 25.0], [0.0, 0.0, 0.1, 1.0]));
        assert_quad(q[1], ([30.0, 5.0, 140.0, 25.0], [0.1, 0.0, 0.65, 1.0]));
        assert_quad(q[2], ([140.0, 5.0, 160.0, 25.0], [0.9, 0.0, 1.0, 1.0]));
        assert_eq!(q.len(), 3);
    }
    #[test]
    fn nine_slice_corners_and_a_stretched_centre() {
        // Faithful 32x's tooltip/frame: 200x200 texels, 100x100, border 12, stretched.
        let n = faithful(
            r#"{"gui":{"scaling":{"type":"nine_slice","width":100,"height":100,"border":12,"stretch_inner":true}}}"#,
        );
        let mut batch = HudBatch::scratch([640, 480]);
        draw_nine_slice(
            &mut batch,
            sprite(200, 200),
            n,
            [0.0, 0.0, 300.0, 40.0],
            u32::MAX,
        );
        let q = quads(&batch);
        assert_eq!(q.len(), 9);
        // Top left corner: 12 GUI pixels, 12/100 of the sprite.
        assert_quad(q[0], ([0.0, 0.0, 12.0, 12.0], [0.0, 0.0, 0.12, 0.12]));
        // The centre is one stretched quad (blitNineSliceInnerSegment, stretch_inner).
        assert_quad(q[7], ([12.0, 12.0, 288.0, 28.0], [0.12, 0.12, 0.88, 0.88]));
        // Bottom right corner.
        assert_quad(q[5], ([288.0, 28.0, 300.0, 40.0], [0.88, 0.88, 1.0, 1.0]));
    }
    #[test]
    fn a_sprite_drawn_at_its_own_size_is_one_quad() {
        let n = NineSlice {
            width: 50,
            height: 50,
            border: [10; 4],
            stretch_inner: false,
        };
        let mut batch = HudBatch::scratch([640, 480]);
        draw_nine_slice(
            &mut batch,
            sprite(100, 100),
            n,
            [4.0, 4.0, 50.0, 50.0],
            u32::MAX,
        );
        let q = quads(&batch);
        assert_eq!(q.len(), 1);
        assert_quad(q[0], ([4.0, 4.0, 54.0, 54.0], [0.0, 0.0, 1.0, 1.0]));
    }
    #[test]
    fn blit_sprite_nine_slices_a_sprite_whose_metadata_says_so() {
        // Vanilla's tooltip/frame: 100x100, border 10, stretched centre.
        let n = faithful(
            r#"{"gui":{"scaling":{"type":"nine_slice","width":100,"height":100,"border":10,"stretch_inner":true}}}"#,
        );
        let mut batch = HudBatch::scratch([640, 480]);
        draw_scaling(
            &mut batch,
            sprite(100, 100),
            Scaling::NineSlice(n),
            [10.0, 10.0, 160.0, 26.0],
            u32::MAX,
        );
        let q = quads(&batch);
        assert_eq!(q.len(), 9);
        // The corners keep their 10 pixels instead of stretching with the box.
        assert_quad(q[0], ([10.0, 10.0, 20.0, 20.0], [0.0, 0.0, 0.1, 0.1]));
        assert_quad(q[5], ([160.0, 26.0, 170.0, 36.0], [0.9, 0.9, 1.0, 1.0]));
    }
    #[test]
    fn blit_sprite_stretches_a_sprite_without_metadata() {
        let mut batch = HudBatch::scratch([640, 480]);
        draw_scaling(
            &mut batch,
            sprite(9, 9),
            Scaling::Stretch,
            [0.0, 0.0, 27.0, 18.0],
            u32::MAX,
        );
        let q = quads(&batch);
        assert_eq!(q.len(), 1);
        assert_quad(q[0], ([0.0, 0.0, 27.0, 18.0], [0.0, 0.0, 1.0, 1.0]));
    }
    #[test]
    fn tile_scaling_reads_its_gui_size() {
        let json = serde_json::json!({"gui": {"scaling": {
            "type": "tile", "width": 16, "height": 8}}});
        assert_eq!(
            parse_tile(&json),
            Some(Scaling::Tile {
                width: 16,
                height: 8
            })
        );
        assert_eq!(parse_scaling(&json), parse_tile(&json));
        // `GuiSpriteScaling.Tile` uses `ExtraCodecs.POSITIVE_INT` for both
        // sides, so zero and out-of-range sizes are invalid metadata.
        for (width, height) in [(0, 8), (16, 0), (i32::MAX as u64 + 1, 8)] {
            let json = serde_json::json!({"gui": {"scaling": {
                "type": "tile", "width": width, "height": height}}});
            assert!(parse_tile(&json).is_none(), "{width} x {height}");
        }
        // A nine-slice is not a tile.
        let json = serde_json::json!({"gui": {"scaling": {
            "type": "nine_slice", "width": 16, "height": 16, "border": 2}}});
        assert!(parse_tile(&json).is_none());
    }
    #[test]
    fn scaling_dispatch_tiles_the_whole_sprite() {
        let mut batch = HudBatch::scratch([640, 480]);
        draw_scaling(
            &mut batch,
            sprite(16, 16),
            Scaling::Tile {
                width: 16,
                height: 16,
            },
            [0.0, 0.0, 34.0, 18.0],
            u32::MAX,
        );
        let q = quads(&batch);
        assert_eq!(q.len(), 6);
        assert_quad(q[0], ([0.0, 0.0, 16.0, 16.0], [0.0, 0.0, 1.0, 1.0]));
        assert_quad(q[1], ([16.0, 0.0, 32.0, 16.0], [0.0, 0.0, 1.0, 1.0]));
        assert_quad(q[2], ([32.0, 0.0, 34.0, 16.0], [0.0, 0.0, 0.125, 1.0]));
        assert_quad(q[5], ([32.0, 16.0, 34.0, 18.0], [0.0, 0.0, 0.125, 0.125]));
    }
    #[test]
    fn scaling_dispatch_keeps_widget_fallback_borders() {
        let rect = sprite(20, 20);
        assert_eq!(
            scaling("gui/sprites/widget/button", &rect, true),
            Scaling::NineSlice(NineSlice {
                width: 20,
                height: 20,
                border: [3; 4],
                stretch_inner: false,
            })
        );
        assert_eq!(
            scaling("gui/sprites/widget/button", &rect, false),
            Scaling::Stretch
        );
        assert_eq!(
            scaling("custom/sprite", &rect, true),
            Scaling::NineSlice(NineSlice {
                width: 20,
                height: 20,
                border: [1; 4],
                stretch_inner: false,
            })
        );
        assert_eq!(scaling("custom/sprite", &rect, false), Scaling::Stretch);
    }
    #[test]
    fn gui_scaling_metadata_reads_each_namespace_with_atlas_keys() {
        use horizon_assets::resource_source::Resource;
        let source = ResourceSource::indexed(std::collections::BTreeMap::from([
            (
                "assets/minecraft/textures/gui/sprites/widget/tile.png.mcmeta".to_owned(),
                Resource::bytes(
                    br#"{"gui":{"scaling":{"type":"tile","width":16,"height":8}}}"#.to_vec(),
                ),
            ),
            (
                "assets/sunfall/textures/gui/sprites/panel/nine.png.mcmeta".to_owned(),
                Resource::bytes(
                    br#"{"gui":{"scaling":{"type":"nine_slice","width":20,"height":20,"border":2}}}"#
                        .to_vec(),
                ),
            ),
            // A sprite whose own path starts with `gui/sprites/` keeps its own
            // atlas key: it must not overwrite the first sprite's metadata.
            (
                "assets/minecraft/textures/gui/sprites/gui/sprites/widget/tile.png.mcmeta"
                    .to_owned(),
                Resource::bytes(
                    br#"{"gui":{"scaling":{"type":"nine_slice","width":20,"height":20,"border":2}}}"#
                        .to_vec(),
                ),
            ),
        ]));
        let metadata = read_metadata(&source);
        let tile = Scaling::Tile {
            width: 16,
            height: 8,
        };
        let nine = Scaling::NineSlice(NineSlice {
            width: 20,
            height: 20,
            border: [2; 4],
            stretch_inner: false,
        });
        assert_eq!(metadata.0.get("gui/sprites/widget/tile"), Some(&tile));
        assert_eq!(
            metadata.0.get("sunfall:gui/sprites/panel/nine"),
            Some(&nine)
        );
        assert_eq!(
            metadata.0.get("gui/sprites/gui/sprites/widget/tile"),
            Some(&nine)
        );
    }
    #[test]
    fn borders_shrink_to_half_the_drawn_size() {
        let n = NineSlice {
            width: 20,
            height: 20,
            border: [8; 4],
            stretch_inner: true,
        };
        let mut batch = HudBatch::scratch([640, 480]);
        // 9 wide: borders of floor(9 / 2) = 4, as vanilla's integer division.
        draw_nine_slice(
            &mut batch,
            sprite(20, 20),
            n,
            [0.0, 0.0, 9.0, 30.0],
            u32::MAX,
        );
        let q = quads(&batch);
        assert_quad(q[0], ([0.0, 0.0, 4.0, 8.0], [0.0, 0.0, 0.2, 0.4]));
        assert_quad(q[2], ([5.0, 0.0, 9.0, 8.0], [0.8, 0.0, 1.0, 0.4]));
    }
    #[test]
    fn append_fields_highlight_the_whole_value_and_blink_their_cursor() {
        use crate::hud::Blend;
        let atlas = HudAtlas::build(&horizon_assets::resource_source::ResourceSource::directory(
            (Path::new("/nonexistent/horizon-test-assets")).to_owned(),
        ))
        .unwrap();
        let end = 10.0 + HudBatch::text_width(&atlas, "abc") + 1.0;
        let span = |b: &HudBatch, r: &std::ops::Range<u32>| {
            let v = &b.vertices[r.start as usize..r.end as usize];
            let xs = v.iter().map(|v| v.pos[0]);
            let ys = v.iter().map(|v| v.pos[1]);
            (
                xs.clone().fold(f32::MAX, f32::min),
                xs.fold(f32::MIN, f32::max),
                ys.clone().fold(f32::MAX, f32::min),
                ys.fold(f32::MIN, f32::max),
            )
        };
        // Select All in the creative search: inverted white, then blue added,
        // from one pixel before the text to the cursor, 11 high.
        let mut b = HudBatch::scratch([854, 480]);
        append_field(
            &mut b,
            &atlas,
            "abc",
            (10.0, 20.0, 80.0),
            (u32::MAX, true),
            50,
            true,
            true,
            0,
        );
        let blends: Vec<_> = b.blends.iter().map(|(_, blend)| *blend).collect();
        assert_eq!(blends, vec![Blend::Invert, Blend::Additive]);
        for (r, _) in &b.blends {
            assert_eq!(span(&b, r), (9.0, end, 19.0, 30.0));
        }
        // The anvil keeps its text colour; 300 ms later the cursor is hidden.
        let mut b = HudBatch::scratch([854, 480]);
        append_field(
            &mut b,
            &atlas,
            "abc",
            (10.0, 20.0, 103.0),
            (u32::MAX, true),
            50,
            true,
            false,
            300,
        );
        let blends: Vec<_> = b.blends.iter().map(|(_, blend)| *blend).collect();
        assert_eq!(blends, vec![Blend::Additive]);
        let quads = |ms| {
            let mut b = HudBatch::scratch([854, 480]);
            append_field(
                &mut b,
                &atlas,
                "abc",
                (10.0, 20.0, 103.0),
                (0, true),
                50,
                false,
                false,
                ms,
            );
            b.vertices.len()
        };
        assert!(quads(300) < quads(600));
        // A full field shows a bar on its last pixel instead of an underscore.
        let mut b = HudBatch::scratch([854, 480]);
        append_field(
            &mut b,
            &atlas,
            "abc",
            (10.0, 20.0, 80.0),
            (u32::MAX, true),
            3,
            false,
            true,
            0,
        );
        let bar = b.vertices.len() as u32 - 6..b.vertices.len() as u32;
        assert_eq!(span(&b, &bar), (end - 1.0, end, 19.0, 30.0));
    }
    #[test]
    fn held_scrollbars_follow_the_pointer_and_stop_at_the_ends() {
        // A 100-pixel list holding 400: the thumb is 25 -> 32 high, so a
        // pixel of drag scrolls 300 / 68 pixels (AbstractScrollArea.mouseDragged).
        let (top, bottom, content) = (0.0, 100.0, 400.0);
        let mut s = Scroll::default();
        assert!(!s.press((50.0, 10.0), 100.0, top, bottom, content));
        assert!(s.press((103.0, 10.0), 100.0, top, bottom, content));
        s.drag(20.0, top, bottom, content);
        assert!((s.amount - 10.0 * 300.0 / 68.0).abs() < 1e-3);
        s.drag(150.0, top, bottom, content);
        assert_eq!(s.amount, 300.0);
        s.drag(-5.0, top, bottom, content);
        assert_eq!(s.amount, 0.0);
        s.release();
        s.drag(50.0, top, bottom, content);
        assert_eq!(s.amount, 0.0);
        // Nothing to scroll: the column doesn't take the press.
        assert!(!s.press((103.0, 10.0), 100.0, top, bottom, 90.0));
        s.wheel(-2.0, 18.0, top, bottom, content);
        assert_eq!(s.amount, 36.0);
    }
    #[test]
    fn edit_box_clicks_place_the_cursor_and_select() {
        let atlas = HudAtlas::build(&horizon_assets::resource_source::ResourceSource::directory(
            (Path::new("/nonexistent/horizon-test-assets")).to_owned(),
        ))
        .unwrap();
        let mut e = EditBox::new(10.0, 0.0, 200.0, 20.0);
        e.set_value("hello world");
        let at = |e: &EditBox, s: &str| e.text_x() + atlas.font.width(s);
        // findClickedPositionInText: the characters left of the pointer.
        e.click(&atlas, at(&e, "hello "), false, false);
        assert_eq!((e.cursor, e.highlight), (6, 6));
        // Shift extends the selection from the anchor.
        e.click(&atlas, at(&e, "hello wor"), true, false);
        assert_eq!(e.selected_text(), "wor");
        // selectWord: the word and the spaces after it.
        e.click(&atlas, at(&e, "he"), false, true);
        assert_eq!(e.selected_text(), "hello ");
        // onDrag from a click: left of the text is its start.
        e.click(&atlas, at(&e, "hello "), false, false);
        e.drag_to(&atlas, e.text_x() - 5.0);
        assert_eq!((e.cursor, e.highlight), (0, 6));
    }
    /// `EditBox.scrollTo`: a cursor on the first shown character steps back a field's width,
    /// so replacing all of a scrolled text shows the new text, and walking left past the
    /// shown start pages back.
    #[test]
    fn edit_box_steps_back_when_the_cursor_reaches_the_first_shown_character() {
        let atlas = HudAtlas::build(&horizon_assets::resource_source::ResourceSource::directory(
            (Path::new("/nonexistent/horizon-test-assets")).to_owned(),
        ))
        .unwrap();
        let mut e = EditBox::new(0.0, 0.0, 60.0, 20.0);
        e.set_value("the quick brown fox jumps over");
        e.layout(&atlas);
        let scrolled = e.display;
        assert!(scrolled > 0);
        assert!(e.key(winit::keyboard::KeyCode::KeyA, true, false));
        e.insert_text("a");
        e.layout(&atlas);
        assert_eq!((e.value.as_str(), e.cursor, e.display), ("a", 1, 0));
        e.set_value("the quick brown fox jumps over");
        e.layout(&atlas);
        e.set_cursor(scrolled, false);
        e.layout(&atlas);
        assert!(e.display < scrolled);
        assert!(e.cursor > e.display);
    }
    #[test]
    fn edit_box_editing() {
        let mut e = EditBox::new(0.0, 0.0, 100.0, 20.0);
        e.max_length = 10;
        e.insert_text("hello world");
        assert_eq!(e.value, "hello worl");
        e.set_cursor(5, false);
        assert_eq!(e.word_position(-1), 0);
        assert_eq!(e.word_position(1), 6);
        e.delete_chars(-2);
        assert_eq!(e.value, "hel worl");
        e.set_cursor(0, false);
        e.set_cursor(3, true);
        assert_eq!(e.selected_text(), "hel");
        e.insert_text("é§x");
        assert_eq!(e.value, "éx worl");
        assert_eq!(e.cursor, 2);
    }
    #[test]
    fn word_position_steps_over_a_word_and_its_spaces() {
        assert_eq!(word_position("hello world", -1, 11), 6);
        assert_eq!(word_position("hello world", -1, 6), 0);
        assert_eq!(word_position("hello   world", -1, 13), 8);
        assert_eq!(word_position("hello   world", 1, 5), 8);
        // From inside a word: back to its start, forward past it and its spaces.
        assert_eq!(word_position("hello world", -1, 8), 6);
        assert_eq!(word_position("hello world", 1, 2), 6);
        // The ends, and a position past the end.
        assert_eq!(word_position("hello", -1, 0), 0);
        assert_eq!(word_position("hello", 1, 5), 5);
        assert_eq!(word_position("hi", -1, 99), 0);
        assert_eq!(word_position("hi", 1, 99), 2);
    }
    #[test]
    fn multiline_words_split_at_any_whitespace() {
        // Ctrl+Backspace at the end of `one\ntwo` deletes `two` only.
        assert_eq!(multiline_word_position("one\ntwo", -1, 7), 4);
        // From the end the search starts at the last character, as vanilla's does.
        assert_eq!(multiline_word_position("one\n", -1, 4), 0);
        assert_eq!(multiline_word_position("a b", -1, 3), 0);
        assert_eq!(multiline_word_position("one\n\ntwo", 1, 0), 5);
        assert_eq!(multiline_word_position("one\u{a0}two", 1, 0), 7);
        assert_eq!(multiline_word_position("", -1, 0), 0);
    }
    #[test]
    fn delete_words_removes_the_word_before_the_end() {
        let mut v = "stone bricks".to_string();
        assert!(delete_words(&mut v, -1));
        assert_eq!(v, "stone ");
        assert!(delete_words(&mut v, -1));
        assert_eq!(v, "");
        assert!(!delete_words(&mut v, -1));
        // Characters, not bytes.
        let mut v = "a é§x".to_string();
        assert!(delete_words(&mut v, -1));
        assert_eq!(v, "a ");
        // Nothing after the end to delete.
        let mut v = "stone".to_string();
        assert!(!delete_words(&mut v, 1));
        assert_eq!(v, "stone");
    }
    #[test]
    fn a_keyboard_focused_tooltip_uses_vanillas_widget_edges() {
        let screen = (320, 240);
        assert_eq!(
            focused_tooltip_position(screen, [7, 160, 98, 20], (170, 40)),
            (10, 184)
        );
        // Exact fit below; one pixel taller goes above.
        assert_eq!(
            focused_tooltip_position(screen, [7, 160, 98, 20], (170, 53)),
            (10, 184)
        );
        assert_eq!(
            focused_tooltip_position(screen, [7, 160, 98, 20], (170, 54)),
            (10, 102)
        );
        // Right overflow aligns against the widget's right edge, not the screen's.
        assert_eq!(
            focused_tooltip_position(screen, [250, 100, 60, 20], (170, 40)),
            (137, 124)
        );
        assert_eq!(
            focused_tooltip_position((120, 240), [7, 160, 98, 20], (170, 40)),
            (4, 184)
        );
    }

    #[test]
    fn a_pad_focused_tooltip_hangs_from_its_widgets_bottom_left() {
        let screen = (427.0, 240.0);
        let widget = [100.0, 100.0, 150.0, 20.0];
        // Frame at 100, 121: lines 4 further in.
        assert_eq!(
            tooltip_under_position(screen, widget, (80.0, 10.0)),
            (104.0, 125.0)
        );
        // The screen's right edge moves it left, its bottom puts it over the widget.
        assert_eq!(
            tooltip_under_position(screen, [400.0, 100.0, 20.0, 20.0], (80.0, 10.0)),
            (343.0, 125.0)
        );
        assert_eq!(
            tooltip_under_position(screen, [100.0, 215.0, 150.0, 20.0], (80.0, 10.0)),
            (104.0, 200.0)
        );
    }
    /// `MenuTooltipPositioner.positionTooltip`: the pointer's 12 px offset,
    /// the 5 px screen margin, and the 3 to 5 px the tooltip keeps from the
    /// widget's top or bottom edge.
    #[test]
    fn a_menu_tooltip_stays_by_its_widget() {
        let screen = (427, 240);
        let widget = [100, 100, 150, 20];
        let size = (80, 10);
        // Pointer at the widget's top edge: the tooltip drops 3 px to 103,
        // then `getOffset` adds round(lerp(3 / 20, 17, 5)) = round(15.2) = 15.
        assert_eq!(
            menu_tooltip_position(screen, (100, 100), widget, size),
            (112, 118)
        );
        // Near the right edge its left side mirrors to the pointer's left.
        assert_eq!(
            menu_tooltip_position(screen, (400, 100), widget, size),
            (308, 118)
        );
        // Halfway down the offset follows the pointer: 113 + round(9.2).
        assert_eq!(
            menu_tooltip_position(screen, (100, 110), widget, size),
            (112, 122)
        );
        // A whole widget height from the edge, the offset is 5.
        assert_eq!(
            menu_tooltip_position(screen, (100, 117), widget, size),
            (112, 125)
        );
        // A widget too close to the bottom: the tooltip goes above it,
        // 203 - (10 + 6) - round(lerp(17 / 20, 17, 5)) = 203 - 16 - 7.
        assert_eq!(
            menu_tooltip_position(screen, (100, 200), [100, 200, 150, 20], size),
            (112, 180)
        );
    }
    #[test]
    fn nine_slice_metadata_reads_resolved_resource_names_without_files() {
        use horizon_assets::resource_source::Resource;
        let source = ResourceSource::indexed(std::collections::BTreeMap::from([
            ("assets/minecraft/textures/gui/sprites/widget/button.png.mcmeta".to_owned(),
             Resource::bytes(br#"{"gui":{"scaling":{"type":"nine_slice","width":20,"height":12,"border":3}}}"#.to_vec())),
            ("assets/minecraft/textures/gui/sprites/broken.png.mcmeta".to_owned(), Resource::bytes(b"invalid".to_vec())),
            ("assets/minecraft/textures/block/stone.png.mcmeta".to_owned(), Resource::bytes(b"{}".to_vec())),
            ("assets/lobby/textures/gui/sprites/tooltip/gold_frame.png.mcmeta".to_owned(),
             Resource::bytes(br#"{"gui":{"scaling":{"type":"nine_slice","width":40,"height":40,"border":6}}}"#.to_vec())),
        ]));
        let metadata = read_metadata(&source);
        assert_eq!(metadata.0.len(), 2);
        let Some(Scaling::NineSlice(button)) = metadata.0.get("gui/sprites/widget/button") else {
            panic!("the button's nine-slice metadata");
        };
        assert_eq!(
            (button.width, button.height, button.border),
            (20, 12, [3; 4])
        );
        // Another namespace's sprite is named with it, as the GUI atlas packs it.
        let Some(Scaling::NineSlice(frame)) =
            metadata.0.get("lobby:gui/sprites/tooltip/gold_frame")
        else {
            panic!("the frame's nine-slice metadata");
        };
        assert_eq!((frame.width, frame.border), (40, [6; 4]));
    }
    /// A tooltip style names its sprites as `TooltipRenderUtil` does, in the GUI atlas's names;
    /// the box draws the background, then the frame, 12 pixels out from the contents, each
    /// stretched when it has no `gui.scaling` (`GuiMetadataSection.DEFAULT`), and nothing
    /// without either sprite.
    #[test]
    fn a_tooltip_style_draws_its_own_background_and_frame() {
        assert_eq!(
            tooltip_style_sprites("minecraft", "fancy"),
            [
                "gui/sprites/tooltip/fancy_background",
                "gui/sprites/tooltip/fancy_frame"
            ]
        );
        let [background, frame] = tooltip_style_sprites("lobby", "gold");
        assert_eq!(
            [background.as_str(), frame.as_str()],
            [
                "lobby:gui/sprites/tooltip/gold_background",
                "lobby:gui/sprites/tooltip/gold_frame"
            ]
        );
        let mut atlas = atlas();
        let mut batch = HudBatch::scratch([640, 480]);
        assert!(!tooltip_sprites(
            &mut batch,
            &atlas,
            [&background, &frame],
            [20.0, 20.0, 50.0, 8.0]
        ));
        assert!(batch.vertices.is_empty());
        // Two 16 × 16 sprites without metadata, in different places on the atlas.
        let region = |u0: f32| Rect {
            u0,
            v0: 0.0,
            u1: u0 + 0.25,
            v1: 0.25,
            w: 16,
            h: 16,
        };
        // A background without its frame would draw a box with no border.
        atlas.sprites.insert(background.clone(), region(0.0));
        assert!(!tooltip_sprites(
            &mut batch,
            &atlas,
            [&background, &frame],
            [20.0, 20.0, 50.0, 8.0]
        ));
        assert!(batch.vertices.is_empty());
        atlas.sprites.insert(frame.clone(), region(0.5));
        assert!(tooltip_sprites(
            &mut batch,
            &atlas,
            [&background, &frame],
            [20.0, 20.0, 50.0, 8.0]
        ));
        // Each is one stretched picture over the whole box, the background under the frame.
        let box_ = [8.0, 8.0, 82.0, 40.0];
        assert_eq!(
            quads(&batch),
            [
                (box_, [0.0, 0.0, 0.25, 0.25]),
                (box_, [0.5, 0.0, 0.75, 0.25])
            ]
        );
    }

    /// `EditBox.maxLength` counts Java `String.length()`: UTF-16 code units.
    /// A pasted emoji takes two of them and is never split (vanilla's
    /// `Character.isHighSurrogate` guard drops the half it would keep).
    #[test]
    fn an_edit_box_maximum_counts_utf16_units() {
        assert_eq!(utf16_len("ab😀"), 4);
        assert_eq!(utf16_prefix("ab😀", 3), "ab");
        assert_eq!(utf16_prefix("ab😀", 4), "ab😀");
        let mut edit = EditBox::new(0.0, 0.0, 200.0, 20.0);
        edit.max_length = 3;
        edit.insert_text("ab😀");
        assert_eq!(edit.value, "ab", "three units: a, b and one of the emoji's");
        edit.set_value("");
        edit.max_length = 4;
        edit.insert_text("ab😀");
        assert_eq!(edit.value, "ab😀", "four units, both of the emoji's");
        // The selection's own units give their room back: with "ab"
        // selected there are two units left, enough for the emoji, not
        // for the emoji and the x.
        edit.set_cursor(0, false);
        edit.set_cursor(2, true);
        edit.insert_text("😀x");
        assert_eq!(edit.value, "😀😀");
    }

    /// `EditBox.setInvertHighlightedTextColor`: a selection inverts its text
    /// unless the box asks it not to (the anvil's rename box).
    #[test]
    fn a_selection_inverts_its_text_unless_the_box_says_not_to() {
        let mut edit = EditBox::new(0.0, 0.0, 100.0, 20.0);
        assert!(edit.invert_highlighted_text_color);
        edit.invert_highlighted_text_color = false;
        edit.set_value("name");
        edit.set_cursor(0, false);
        edit.set_cursor(4, true);
        assert_eq!(edit.selected_text(), "name");
        assert!(!edit.invert_highlighted_text_color);
    }

    /// `EditBox.setEditable(false)`: the box keeps the focus, the caret keys
    /// and copy, but `charTyped`/paste/Backspace/Delete do nothing, and Cut
    /// copies without taking the text out. A value and the caret still show,
    /// in `textColorUneditable`.
    #[test]
    fn a_read_only_edit_box_keeps_the_caret_and_copy_but_refuses_editing() {
        use crate::clipboard::{EditShortcut, MemoryClipboard};
        let mut edit = EditBox::new(0.0, 0.0, 100.0, 20.0);
        edit.set_value("name");
        edit.editable = false;
        let mut clipboard = MemoryClipboard("pasted".into());
        // The caret keys still work.
        assert!(edit.key(winit::keyboard::KeyCode::ArrowLeft, false, false));
        assert_eq!(edit.cursor, 3);
        assert!(edit.key(winit::keyboard::KeyCode::Home, false, false));
        assert_eq!(edit.cursor, 0);
        // Backspace and Delete are consumed but change nothing.
        assert!(edit.key(winit::keyboard::KeyCode::Backspace, false, false));
        assert!(edit.key(winit::keyboard::KeyCode::Delete, false, false));
        assert_eq!(edit.value, "name");
        // Typed text (and an IME commit) goes through insert_text: refused.
        edit.insert_text("typed");
        assert_eq!(edit.value, "name");
        edit.delete_chars(-1);
        assert_eq!(edit.value, "name");
        // Paste changes nothing; Cut copies and keeps the text; Copy copies.
        edit.set_cursor(0, false);
        edit.set_cursor(4, true);
        assert!(!edit.shortcut(EditShortcut::Paste, &mut clipboard));
        assert_eq!(edit.value, "name");
        assert!(!edit.shortcut(EditShortcut::Cut, &mut clipboard));
        assert_eq!(edit.value, "name");
        assert_eq!(clipboard.0, "name");
        edit.set_cursor(0, false);
        edit.set_cursor(4, true);
        assert!(!edit.shortcut(EditShortcut::Copy, &mut clipboard));
        assert_eq!(clipboard.0, "name");
        // An editable box does all of it again. (Set the caret past the
        // selection first: insertion replaces a selection.)
        edit.editable = true;
        edit.set_cursor(4, false);
        edit.insert_text("x");
        assert_eq!(edit.value, "namex");
        assert!(edit.key(winit::keyboard::KeyCode::Backspace, false, false));
        assert_eq!(edit.value, "name");
    }

    /// `EditBox.setCentered` (`BookSignScreen`'s title box): with the flag the
    /// text starts `(width - text width) / 2` in, which is also where a click
    /// maps; without it the text starts at the box's left edge.
    #[test]
    fn a_centred_box_starts_its_text_in_the_middle() {
        let mut edit = EditBox::new(10.0, 0.0, 114.0, 9.0);
        edit.bordered = false;
        assert_eq!(edit.text_start_x(0.0), 10.0);
        edit.centered = true;
        assert_eq!(edit.text_start_x(0.0), 67.0);
        assert_eq!(edit.text_start_x(18.0), 58.0);
        // (114 - 5) / 2 = 54.5, rounded down.
        assert_eq!(edit.text_start_x(5.0), 64.0);
    }
    /// A list shorter than the scroller minimum keeps the Java clamp: 20 px of
    /// list holding 40 px of content has a 12 px thumb, not a panic.
    #[test]
    fn a_short_list_keeps_a_whole_scroller() {
        assert_eq!(scrollbar_thumb(10.0, 30.0, 40.0, 0.0), Some((10.0, 12.0)));
        assert_eq!(scrollbar_thumb(10.0, 30.0, 40.0, 10.0), Some((14.0, 12.0)));
        assert_eq!(scrollbar_thumb(10.0, 30.0, 40.0, 20.0), Some((18.0, 12.0)));
        let mut scroll = Scroll::default();
        assert!(scroll.press((0.0, 10.0), 0.0, 10.0, 30.0, 40.0));
        scroll.drag(20.0, 10.0, 30.0, 40.0);
        assert_eq!(scroll.amount, 20.0);
    }
}
