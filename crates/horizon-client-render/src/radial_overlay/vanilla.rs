//! The dial on vanilla's own surfaces (`menu_style::Surfaces::Vanilla`, the look with no style
//! given, as the settings pages): every sprite is vanilla's, read from the player's resource
//! packs, every line of text is vanilla's font with its shadow, on a vanilla plate wherever it
//! would sit on the world.
//!
//! - The slots are advancement task frames (`advancements/task_frame_*`, square) with their
//!   item icons. The picked one wears the obtained (gold) frame under the glow a container
//!   draws on a hovered slot (`container/slot_highlight_*`); one that can't run is dimmed, frame
//!   and icon (vanilla has no slash); an empty one is a faded frame. A view too tight for a
//!   frame takes vanilla's 18 px inventory slot instead, before the HUD's rows are covered.
//! - The hub is vanilla's tooltip (`ui::tooltip_panel`) with one short line: the picked item's
//!   name. No state (a toggle's included), reasons, hints or second lines.
//! - The pages are Create World's tabs (`graphics_style::vanilla_tab`), the bumpers beside them.
//! - The Cancel hint sits on the text background, as chat and subtitles do (the player's Text
//!   Background Opacity).
//!
//! The geometry stays the dial's [`Layout`]: only the header's height, the tile (a frame, 26)
//! and the absence of chips differ.
use super::*;
use crate::{graphics_style, text::Text, ui};

/// Vanilla's text colours: white, and `§7` grey.
pub(super) const WHITE: u32 = colour(0xFFFFFF, 255);
pub(super) const GREY: u32 = colour(0xAAAAAA, 255);
/// A light shade behind the ring, as a menu over the world dims it (the tooltip and the plates
/// carry the text; this only says the dial is open). Centre, at `VIGNETTE_MID`.
pub(super) const SHADE: [u8; 2] = [110, 70];

/// Where a frame doesn't fit: vanilla's inventory slot, its item 1 px in, and the slot
/// highlight a container draws round a slot under the cursor (behind and over the item, 4 px
/// round it).
pub const SMALL_CELL: f32 = 18.0;
pub(super) const SLOT: &str = "gui/sprites/container/slot";
const HIGHLIGHT_BACK: &str = "gui/sprites/container/slot_highlight_back";
const HIGHLIGHT_FRONT: &str = "gui/sprites/container/slot_highlight_front";
const HIGHLIGHT: f32 = 24.0;
const HIGHLIGHT_OUT: f32 = 4.0;
/// The tab row on vanilla's surfaces: vanilla's tab bar (`MenuTabBar.HEIGHT`). A small view
/// keeps the same sprites a little shorter: the label still centres in the unselected art.
pub const TAB_H_VANILLA: f32 = 24.0;
pub const TAB_H_VANILLA_SMALL: f32 = 18.0;
/// Every tab: the widest label plus this, between these widths (kept even).
const TAB_PAD: f32 = 16.0;
const TAB_W: [f32; 2] = [40.0, 96.0];
/// Vanilla's tooltip lines: the first, 2 px more, then one every 10.
const TIP_FIRST: f32 = 12.0;
const TIP_STEP: f32 = 10.0;
/// How far vanilla's tooltip shows past its text (its sprites reach 12, 9 of them clear).
const TIP_EDGE: f32 = 3.0;
/// The text background's margin round a line (`ChatComponent`: 1 above, 1 right, 2 left).
const PLATE_PAD: f32 = 2.0;

/// An advancement task frame, square, 26 px; its item 5 px in.
pub const FRAME: f32 = 26.0;
const FRAME_INSET: f32 = 5.0;
pub(super) const TASK_FRAME: [&str; 2] = [
    "gui/sprites/advancements/task_frame_unobtained",
    "gui/sprites/advancements/task_frame_obtained",
];
/// A slot that can't run, or an empty one: its frame or inventory slot at half alpha.
pub(super) const FADED_FRAME: u32 = colour(0xFFFFFF, 128);

/// The smallest tile the ring takes on vanilla's surfaces: a whole frame.
pub fn least_tile() -> f32 {
    FRAME
}

/// Whether the dial draws on vanilla's surfaces.
pub fn on() -> bool {
    graphics_style::vanilla_surfaces()
}

/// The tab row's height.
pub fn header_h(roomy: bool) -> f32 {
    if roomy {
        TAB_H_VANILLA
    } else {
        TAB_H_VANILLA_SMALL
    }
}

pub fn draw(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    painter: &dyn IconPainter,
    dial: &Dial,
    look: &Look,
    clock: Clock,
) {
    let lay = Layout::new(batch.width, batch.height, dial.room);
    let (alpha, scale) = look.presence(clock);
    let page = dial.pages.page(dial.seen.page);
    let start = batch.vertices.len();
    batch.fill_disc_gradient(
        atlas,
        [lay.cx, lay.cy],
        lay.r + lay.tile + VIGNETTE_REACH,
        &[
            Stop(0.0, colour(0, SHADE[0])),
            Stop(VIGNETTE_MID, colour(0, SHADE[1])),
            Stop(1.0, colour(0, 0)),
        ],
    );
    let tiles = batch.vertices.len();
    draw_cells(batch, atlas, painter, &lay, dial, page, look, clock);
    let slide = look.slide(clock);
    batch.translate_since(tiles, slide, 0.0);
    batch.alpha_since(tiles, 1.0 - slide.abs() / TURN_FADE_PX);
    draw_tooltip(batch, atlas, &lay, dial, page);
    batch.scale_since(start, lay.cx, lay.cy, scale, scale);
    let header = batch.vertices.len();
    draw_tabs(batch, atlas, &lay, dial);
    draw_cancel(batch, atlas, &lay, dial);
    batch.clip_since(header, 0.0, 0.0, lay.w, lay.h);
    batch.alpha_since(start, alpha);
}

/// The eight slots: each a frame with its icon, the picked one gold under the hover glow, one
/// that can't run dimmed, an empty one faded.
#[allow(clippy::too_many_arguments)]
fn draw_cells(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    painter: &dyn IconPainter,
    lay: &Layout,
    dial: &Dial,
    page: &Page,
    look: &Look,
    clock: Clock,
) {
    let flash = look.flash(clock);
    let small = lay.tile < least_tile();
    // The item's place: 5 px into a frame, 1 into an inventory slot.
    let inset = if small { 1.0 } else { FRAME_INSET };
    for slot in 0..SLOTS {
        let item = page.slot(slot);
        let [cx, cy] = lay.centre(slot);
        let [x, y, size, _] = cell_rect(lay, slot);
        let chosen = dial.seen.selected == Some(slot) && !dial.seen.waiting;
        let live = dial.available[slot];
        let (ix, iy) = (x + inset, y + inset);
        let around = [ix - HIGHLIGHT_OUT, iy - HIGHLIGHT_OUT];
        // An empty slot, or one that can't run, is faded: a frame or an inventory slot alike.
        let faded = item.is_empty() || !live;
        let tint = if faded { FADED_FRAME } else { u32::MAX };
        let sprite = if small {
            SLOT
        } else {
            TASK_FRAME[chosen as usize]
        };
        ui::blit_sprite(batch, atlas, sprite, x, y, size, size, tint);
        if chosen {
            let [hx, hy] = around;
            ui::blit_sprite(
                batch,
                atlas,
                HIGHLIGHT_BACK,
                hx,
                hy,
                HIGHLIGHT,
                HIGHLIGHT,
                u32::MAX,
            );
        }
        if let Some((_, strength)) = flash.filter(|(s, _)| *s == slot) {
            batch.fill(atlas, ix, iy, ICON, ICON, shapes::faded(FLASH, strength));
        }
        let mut dim = if live { 1.0 } else { DIM };
        if dial.seen.waiting {
            dim *= WAITING_DIM;
        }
        match item {
            Slot::Action(action) => {
                icon::draw(batch, atlas, painter, icon(*action), [ix, iy, ICON], dim);
            }
            Slot::Missing(_) => {
                let mw = atlas.font.width(MISSING_MARK);
                let (mx, my) = ((cx - mw / 2.0).floor(), (cy - LINE_H / 2.0).ceil());
                let mark = shapes::faded(GREY, dim.max(DIM));
                batch.text(atlas, MISSING_MARK, mx, my, mark, true);
            }
            Slot::Empty => {}
        }
        if chosen {
            let [hx, hy] = around;
            ui::blit_sprite(
                batch,
                atlas,
                HIGHLIGHT_FRONT,
                hx,
                hy,
                HIGHLIGHT,
                HIGHLIGHT,
                u32::MAX,
            );
        }
    }
}

/// The hub tooltip's one line: what the picked item is, its name, cut as vanilla cuts a line too
/// wide for its room. Never a state, even a toggle's: players know what is on, and a bare "Off"
/// says nothing of what it belongs to. Nothing for an empty slot or while the stick must be let
/// go: the hub says no more (less is more).
pub(super) fn tooltip_lines(
    atlas: &HudAtlas,
    lay: &Layout,
    dial: &Dial,
    page: &Page,
) -> Vec<(String, u32)> {
    let Some(slot) = dial.seen.selected.filter(|_| !dial.seen.waiting) else {
        return Vec::new();
    };
    let text = match page.slot(slot) {
        Slot::Empty => return Vec::new(),
        Slot::Missing(_) => lang::get(MISSING),
        Slot::Action(action) => action_label(*action),
    };
    let text = fit::clip_text(atlas, &text, tooltip_room(lay)).into_owned();
    vec![(text, WHITE)]
}

/// How wide the tooltip's text may be.
pub(super) fn tooltip_room(lay: &Layout) -> f32 {
    // Its frame then ends where the hub does: 1 px clear of the cells' grown squares.
    2.0 * (lay.hub - TIP_EDGE + HUB_GAP - GROW - 1.0)
}

/// The hub: the selection in vanilla's tooltip, centred.
fn draw_tooltip(batch: &mut HudBatch, atlas: &HudAtlas, lay: &Layout, dial: &Dial, page: &Page) {
    let lines = tooltip_lines(atlas, lay, dial, page);
    let Some(rect) = tooltip_rect(atlas, lay, &lines) else {
        return;
    };
    ui::tooltip_panel(batch, atlas, rect);
    let [x, mut ly, _, _] = rect;
    for (i, (text, c)) in lines.iter().enumerate() {
        batch.text(atlas, text, x, ly, *c, true);
        ly += if i == 0 { TIP_FIRST } else { TIP_STEP };
    }
}

/// The tooltip's text area, centred on the hub (vanilla's tooltip: the first line, 2 px more,
/// then a line every 10); `None` with nothing to say.
pub(super) fn tooltip_rect(
    atlas: &HudAtlas,
    lay: &Layout,
    lines: &[(String, u32)],
) -> Option<[f32; 4]> {
    if lines.is_empty() {
        return None;
    }
    let w = lines
        .iter()
        .map(|(t, _)| atlas.font.width(t).ceil())
        .fold(0.0, f32::max);
    let more = lines.len().saturating_sub(1) as f32;
    let h = LINE_H - 1.0
        + if more > 0.0 {
            TIP_FIRST - TIP_STEP + more * TIP_STEP
        } else {
            0.0
        };
    Some([(lay.cx - w / 2.0).floor(), (lay.cy - h / 2.0).floor(), w, h])
}

/// What the tooltip covers: its text area and the frame round it.
#[cfg(test)]
pub(super) fn tooltip_shown([x, y, w, h]: [f32; 4]) -> [f32; 4] {
    [
        x - TIP_EDGE,
        y - TIP_EDGE,
        w + 2.0 * TIP_EDGE,
        h + 2.0 * TIP_EDGE,
    ]
}

/// Slot `slot`'s cell as drawn, `[x, y, w, h]`: a frame, or an inventory slot where the layout's
/// tiles are that small.
pub(super) fn cell_rect(lay: &Layout, slot: usize) -> [f32; 4] {
    let size = if lay.tile < least_tile() {
        SMALL_CELL
    } else {
        FRAME
    };
    let [cx, cy] = lay.centre(slot);
    [
        (cx - size / 2.0).round(),
        (cy - size / 2.0).round(),
        size,
        size,
    ]
}

/// Which pages the row shows: as many round the open one as fit `room`, left to right in the
/// order the bumpers turn.
fn shown(n: usize, open: usize, room: f32, width: impl Fn(usize) -> f32) -> Vec<usize> {
    let mut shown = vec![open];
    let mut used = width(open);
    'grow: for step in 1..n {
        for i in [(open + n - step) % n, (open + step) % n] {
            if shown.contains(&i) {
                continue;
            }
            if used + width(i) > room {
                break 'grow;
            }
            used += width(i);
            shown.push(i);
        }
    }
    // Back pages left of the open one: by distance going back, wrapping.
    shown.sort_by_key(|i| (i + n - open + n / 2) % n);
    shown
}

/// The tabs the row shows, `(page, [x, y, w, h])` left to right: as vanilla's tab bar has them
/// (`TabNavigationBar`), every tab the same width, here the widest label's plus `TAB_PAD`, kept
/// even, so with an odd number shown the open tab's middle is the dial's; the row is centred on it.
pub(super) fn tab_rects(atlas: &HudAtlas, lay: &Layout, dial: &Dial) -> Vec<(usize, [f32; 4])> {
    let n = dial.pages.len();
    let widest = (0..n)
        .map(|i| atlas.font.width(&page_label(dial.pages.page(i))).ceil())
        .fold(0.0, f32::max);
    let tw = ((widest + TAB_PAD).clamp(TAB_W[0], TAB_W[1]) / 2.0).ceil() * 2.0;
    let [_, y, w, h] = lay.header();
    let keys = if n > 1 { dial.page_keys } else { [None, None] };
    let key_w = keys.map(|k| k.map_or(0.0, |g| pad_glyph::width(atlas, g) + GLYPH_GAP));
    let room = w - 2.0 * (SIDE_MARGIN + key_w[0].max(key_w[1]));
    let tabs = shown(n, dial.seen.page, room, |_| tw);
    let left = ((w - tw * tabs.len() as f32) / 2.0).floor();
    tabs.into_iter()
        .enumerate()
        .map(|(k, i)| (i, [left + tw * k as f32, y, tw, h]))
        .collect()
}

/// The pages as Create World's tabs, the bumpers that turn them at the row's ends.
fn draw_tabs(batch: &mut HudBatch, atlas: &HudAtlas, lay: &Layout, dial: &Dial) {
    let tabs = tab_rects(atlas, lay, dial);
    for (i, rect) in &tabs {
        let label = Text::literal(page_label(dial.pages.page(*i)));
        let open = *i == dial.seen.page;
        graphics_style::vanilla_tab(batch, atlas, *rect, &label, (open, false, true));
    }
    let (Some((_, first)), Some((_, last))) = (tabs.first(), tabs.last()) else {
        return;
    };
    let keys = if dial.pages.len() > 1 {
        dial.page_keys
    } else {
        [None, None]
    };
    let [left, y, _, h] = *first;
    let right = last[0] + last[2];
    // The bumpers on the line of the labels of the tabs that aren't open.
    let text_y = graphics_style::vanilla_tab_label_y(y, h, false);
    if let Some(back) = keys[0] {
        let kw = pad_glyph::width(atlas, back) + GLYPH_GAP;
        pad_glyph::draw(batch, atlas, back, left - kw + 1.0, text_y);
    }
    if let Some(on) = keys[1] {
        pad_glyph::draw(batch, atlas, on, right + GLYPH_GAP + 1.0, text_y);
    }
}

/// B's Cancel on the text background, where the dial's own look has its plate.
fn draw_cancel(batch: &mut HudBatch, atlas: &HudAtlas, lay: &Layout, dial: &Dial) {
    let row = HintRow::new([dial.cancel], lang::get("gui.cancel"));
    let look = hint::Look {
        color: WHITE,
        shadow: true,
        ..hint::NOTICE
    };
    let w = (HINT_PAD + row.measure(atlas, &look) + HINT_PAD).ceil();
    let Some([x, y, w, h]) = lay.hint(w, &[]) else {
        return;
    };
    let ty = y + ((h - LINE_H + 1.0) / 2.0).floor();
    // The plate round the caption's line; the glyph's art reaches past it, as in the guide.
    batch.fill(
        atlas,
        x,
        ty - PLATE_PAD,
        w,
        LINE_H + 2.0 * PLATE_PAD,
        colour(0, dial.backing),
    );
    hint::draw_hint(batch, atlas, &row, &look, x + HINT_PAD, ty);
}
