//! The settings pages on vanilla's own surfaces (`menu_style::Surfaces::Vanilla`, the look with
//! no style given): what vanilla's Options screens draw, from the player's resource packs, so a
//! pack that restyles vanilla's menus restyles these too. The pages keep their own layout; only
//! the paint changes.
use super::{WHITE, scrolling_label};
use crate::{
    hud::{HudAtlas, HudBatch, rgba},
    text::Text,
    ui,
};

const BUTTON: &str = "gui/sprites/widget/button";
const BUTTON_HIGHLIGHTED: &str = "gui/sprites/widget/button_highlighted";
const BUTTON_DISABLED: &str = "gui/sprites/widget/button_disabled";
const CHECKBOX: &str = "gui/sprites/widget/checkbox";
const CHECKBOX_SELECTED: &str = "gui/sprites/widget/checkbox_selected";
const CHECKBOX_HIGHLIGHTED: &str = "gui/sprites/widget/checkbox_highlighted";
const CHECKBOX_SELECTED_HIGHLIGHTED: &str = "gui/sprites/widget/checkbox_selected_highlighted";
const SLIDER: &str = "gui/sprites/widget/slider";
const SLIDER_HIGHLIGHTED: &str = "gui/sprites/widget/slider_highlighted";
const HANDLE: &str = "gui/sprites/widget/slider_handle";
const HANDLE_HIGHLIGHTED: &str = "gui/sprites/widget/slider_handle_highlighted";
const TEXT_FIELD: &str = "gui/sprites/widget/text_field";
const TAB: &str = "gui/sprites/widget/tab";
const TAB_HIGHLIGHTED: &str = "gui/sprites/widget/tab_highlighted";
const TAB_SELECTED: &str = "gui/sprites/widget/tab_selected";
const TAB_SELECTED_HIGHLIGHTED: &str = "gui/sprites/widget/tab_selected_highlighted";
/// The font's line height, as vanilla centres text on it (`Font.lineHeight`).
const LINE_H: f32 = 9.0;
/// A tab's label sits this much lower while the tab isn't open (`MenuTabButton.SELECTED_OFFSET`).
const TAB_DROP: f32 = 3.0;
const SCROLLER: &str = "gui/sprites/widget/scroller";
const SCROLLER_BACKGROUND: &str = "gui/sprites/widget/scroller_background";

/// Vanilla's grey for a control that can't be used (`AbstractWidget`'s inactive text).
const INACTIVE: u32 = rgba(160, 160, 160, 255);
/// A tick box's side: vanilla's `Checkbox.getBoxSize`, the font's line (9) plus 8.
const BOX: f32 = 17.0;
/// A slider's handle width (`AbstractSliderButton`).
const HANDLE_W: f32 = 8.0;
/// A control that can't be used is drawn at this alpha, as vanilla's `active = false` widgets.
const FADED: u32 = rgba(255, 255, 255, 128);
/// A button's label from its edges: `AbstractButton`'s 2 px a side, and room to breathe.
pub(super) const LABEL_PAD: f32 = 12.0;
/// The list background's tile and a separator's, in GUI pixels (`Screen`, `HeaderAndFooterLayout`).
const LIST_TILE: f32 = 32.0;
const SEPARATOR: [f32; 2] = [32.0, 2.0];
/// A selection list's entry: grey edge while it isn't focused, inside black
/// (`AbstractSelectionList.renderSelection`).
const SELECTED_EDGE: u32 = rgba(128, 128, 128, 255);
const SELECTED_FILL: u32 = rgba(0, 0, 0, 255);

fn text(live: bool) -> u32 {
    if live { WHITE } else { INACTIVE }
}

fn list_sprites(in_world: bool) -> (&'static str, &'static str, &'static str) {
    if in_world {
        (
            "gui/inworld_menu_list_background",
            "gui/inworld_header_separator",
            "gui/inworld_footer_separator",
        )
    } else {
        (
            "gui/menu_list_background",
            "gui/header_separator",
            "gui/footer_separator",
        )
    }
}

/// A panel as vanilla's lists are drawn: the list background, the header's separator over it and
/// the footer's under it.
pub(super) fn sheet(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    [x, y, w, h]: [f32; 4],
    in_world: bool,
) {
    let (background, header, footer) = list_sprites(in_world);
    ui::tiled(batch, atlas, background, x, y, w, h, LIST_TILE, u32::MAX);
    ui::tiled_sized(
        batch,
        atlas,
        header,
        x,
        y - 2.0,
        w,
        2.0,
        SEPARATOR,
        u32::MAX,
    );
    ui::tiled_sized(batch, atlas, footer, x, y + h, w, 2.0, SEPARATOR, u32::MAX);
}

/// A rule as vanilla's header separator, 2 px from the rule's top.
pub(super) fn separator(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    [x, y, w, _]: [f32; 4],
    in_world: bool,
) {
    let (_, header, _) = list_sprites(in_world);
    ui::tiled_sized(batch, atlas, header, x, y, w, 2.0, SEPARATOR, u32::MAX);
}

/// A selection list's chosen entry: black inside a grey edge.
pub(super) fn selected(batch: &mut HudBatch, atlas: &HudAtlas, rect: [f32; 4]) {
    let [x, y, w, h] = rect;
    super::fill(batch, atlas, rect, SELECTED_EDGE);
    super::fill(
        batch,
        atlas,
        [x + 1.0, y + 1.0, w - 2.0, h - 2.0],
        SELECTED_FILL,
    );
}

/// A row pointed at or focused: a selection list's grey edge round it.
pub(super) fn outline(batch: &mut HudBatch, atlas: &HudAtlas, rect: [f32; 4]) {
    super::ring(batch, atlas, rect, SELECTED_EDGE);
}

/// Vanilla's scroller: its track and thumb sprites.
pub(super) fn scroller(batch: &mut HudBatch, atlas: &HudAtlas, track: [f32; 4], thumb: [f32; 4]) {
    let [tx, ty, tw, th] = track;
    ui::nine_slice(batch, atlas, SCROLLER_BACKGROUND, tx, ty, tw, th, u32::MAX);
    let [x, y, w, h] = thumb;
    ui::nine_slice(batch, atlas, SCROLLER, x, y, w, h, u32::MAX);
}

/// A button: vanilla's sprite, its label centred (or slid when long) in white, grey when off.
pub(super) fn button(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    rect: [f32; 4],
    label: &Text,
    (hover, live): (bool, bool),
    room: f32,
) {
    let [x, y, w, h] = rect;
    let sprite = match (live, hover) {
        (false, _) => BUTTON_DISABLED,
        (true, true) => BUTTON_HIGHLIGHTED,
        (true, false) => BUTTON,
    };
    ui::nine_slice(batch, atlas, sprite, x, y, w, h, u32::MAX);
    let ty = y + ((h - 8.0) / 2.0).floor();
    let left = x + 2.0 + room;
    scrolling_label(
        batch,
        atlas,
        label,
        [left, ty, w - 4.0 - room],
        (y, h),
        text(live),
    );
}

/// A tick box ending at `right`, centred in the row at `y`; lit while pointed at or held.
pub(super) fn checkbox(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    [right, y]: [f32; 2],
    on: bool,
    (lit, live): (bool, bool),
) {
    let sprite = match (on, lit && live) {
        (true, true) => CHECKBOX_SELECTED_HIGHLIGHTED,
        (true, false) => CHECKBOX_SELECTED,
        (false, true) => CHECKBOX_HIGHLIGHTED,
        (false, false) => CHECKBOX,
    };
    let top = y + ((super::ROW - BOX) / 2.0).floor();
    let tint = if live { u32::MAX } else { FADED };
    ui::blit_sprite(batch, atlas, sprite, right - BOX, top, BOX, BOX, tint);
}

/// A slider: vanilla's track with its value centred, and the handle at `position`.
pub(super) fn slider(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    rect: [f32; 4],
    value: &Text,
    position: f32,
    (hover, live): (bool, bool),
) {
    let [x, y, w, h] = rect;
    let tint = if live { u32::MAX } else { FADED };
    let track = if hover && live {
        SLIDER_HIGHLIGHTED
    } else {
        SLIDER
    };
    ui::nine_slice(batch, atlas, track, x, y, w, h, tint);
    let at = x + (position.clamp(0.0, 1.0) * (w - HANDLE_W)).round();
    let handle = if hover && live {
        HANDLE_HIGHLIGHTED
    } else {
        HANDLE
    };
    ui::nine_slice(batch, atlas, handle, at, y, HANDLE_W, h, tint);
    let ty = y + ((h - 8.0) / 2.0).floor();
    scrolling_label(
        batch,
        atlas,
        value,
        [x + 2.0, ty, w - 4.0],
        (y, h),
        text(live),
    );
}

/// A value a press steps through: a vanilla button showing the value, as its `CycleButton`.
pub(super) fn cycler(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    rect: [f32; 4],
    value: &Text,
    (hover, live): (bool, bool),
) {
    button(batch, atlas, rect, value, (hover, live), 0.0);
}

/// A tab (`MenuTabBar.MenuTabButton`): its sprite; open, the menu background inside it and a
/// 1 px underline 2 px over its bottom, as wide as the label; the label centred.
pub(super) fn tab(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    [x, y, w, h]: [f32; 4],
    label: &Text,
    (selected, lit, live): (bool, bool, bool),
) {
    let sprite = match (selected, lit) {
        (true, true) => TAB_SELECTED_HIGHLIGHTED,
        (true, false) => TAB_SELECTED,
        (false, true) => TAB_HIGHLIGHTED,
        (false, false) => TAB,
    };
    ui::nine_slice(batch, atlas, sprite, x, y, w, h, u32::MAX);
    let colour = text(live);
    if selected {
        ui::menu_background(batch, atlas, x + 2.0, y + 2.0, w - 4.0, h - 2.0, false);
        let lw = atlas.font.width(&label.plain()).min(w - 4.0);
        let lx = x + ((w - lw) / 2.0).floor();
        super::fill(batch, atlas, [lx, y + h - 2.0, lw, 1.0], colour);
    }
    let top = y + if selected { 0.0 } else { TAB_DROP };
    let ty = tab_label_y(y, h, selected);
    scrolling_label(
        batch,
        atlas,
        label,
        [x + 1.0, ty, w - 2.0],
        (top, y + h - top),
        colour,
    );
}

/// Where vanilla centres a tab's label (`MenuTabButton.renderLabel`, through
/// `ActiveTextCollector`): from the tab's top, or `TAB_DROP` under it while it isn't open, to its
/// bottom, on a 9 px line, then 1 px down.
pub(super) fn tab_label_y(y: f32, h: f32, open: bool) -> f32 {
    let top = y + if open { 0.0 } else { TAB_DROP };
    ((top + y + h - LINE_H) / 2.0).floor() + 1.0
}

/// A field: vanilla's text field, its focus ring left to the caller.
pub(super) fn well(batch: &mut HudBatch, atlas: &HudAtlas, [x, y, w, h]: [f32; 4]) {
    ui::nine_slice(batch, atlas, TEXT_FIELD, x, y, w, h, u32::MAX);
}
