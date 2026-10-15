//! The options screen in Horizon's menu style, laid out like the launcher's
//! Settings page: a sidebar with the sliding selection (one item per
//! section), then the section's page header and settings rows (label column,
//! value, control on the right) between 1px dividers.
//!
//! Adding settings (for example the Simple/Advanced graphics menu): add rows
//! to a section in [`SECTIONS`], or a new section. A row is an [`HRow`]: an
//! existing option (`Opt`, drawn as a switch, a slider or a value button from
//! its kind), a header, or a button opening one of vanilla's pages. New kinds
//! of control belong in `ui_theme` so the launcher-matching look stays in one
//! place. Everything else (sliders dragging, scrolling, Done) is handled here.
use crate::{
    extras,
    graphics_style::Cost,
    graphics_ui::{self, Item, Level},
    hud::{HudAtlas, HudBatch},
    options_ui::{Event, Kind, Opt, OptionsUi, Page, apply_preset},
    settings::{SOUND_SOURCES, Settings},
    ui_theme::{self as t, CONTROL, Kind as Button, MenuStyle, ROW_LABEL, SIDEBAR, Ui, color, id},
};

/// One row of a section.
#[derive(Clone, Copy, Debug)]
pub enum HRow {
    Header(&'static str),
    Opt(Opt),
    /// A label, a value and a button opening one of vanilla's pages.
    Page(&'static str, &'static str, Page),
    /// Menu style: Horizon / Classic.
    Style,
    /// Graphics settings level: Simple / Advanced.
    Level,
    /// A graphics setting with its frame-time cost and description.
    Graphics(Item, Cost, &'static str),
    /// One of Horizon's extras (`extras`), off by default.
    Extra(extras::Item),
}
/// A sidebar section: name, description, rows.
pub struct Section {
    pub name: &'static str,
    pub blurb: &'static str,
    pub rows: fn(&Settings) -> Vec<HRow>,
}
pub const SECTIONS: &[Section] = &[
    Section {
        name: "Video",
        blurb: "Graphics, Simple or Advanced: each setting says what it costs.",
        rows: |s| {
            let level = if s.horizon.advanced {
                Level::Advanced
            } else {
                Level::Simple
            };
            let mut r = vec![HRow::Level];
            r.extend(graphics_ui::rows(level).into_iter().map(|row| match row {
                graphics_ui::Row::Header(name) => HRow::Header(name),
                graphics_ui::Row::Setting(g) => HRow::Graphics(g.item, g.cost, g.text),
            }));
            r
        },
    },
    Section {
        name: "Audio",
        blurb: "Volume for every kind of sound, and subtitles.",
        rows: |_| {
            let mut r: Vec<HRow> = (0..SOUND_SOURCES.len())
                .map(|i| HRow::Opt(Opt::Volume(i)))
                .collect();
            r.push(HRow::Opt(Opt::ShowSubtitles));
            r
        },
    },
    Section {
        name: "Controls",
        blurb: "Keys, the mouse, controllers and how movement keys behave.",
        rows: |_| {
            let mut r = vec![HRow::Page(
                "Key binds",
                "Every action and its key",
                Page::KeyBinds,
            )];
            r.push(HRow::Header("Mouse"));
            r.extend(
                [
                    Opt::Sensitivity,
                    Opt::WheelSensitivity,
                    Opt::InvertY,
                    Opt::DiscreteScroll,
                ]
                .map(HRow::Opt),
            );
            r.push(HRow::Header("Movement"));
            r.extend([Opt::ToggleCrouch, Opt::ToggleSprint, Opt::AutoJump].map(HRow::Opt));
            r.push(HRow::Header("Controller"));
            r.push(HRow::Page(
                "Controller binds",
                "Every action and its button",
                Page::PadBinds,
            ));
            r.extend(
                [
                    Opt::PadEnabled,
                    Opt::PadVibration,
                    Opt::PadLookSensitivity,
                    Opt::PadToggleSneak,
                    Opt::PadInvertX,
                    Opt::PadInvertY,
                    Opt::PadMoveDeadzone,
                    Opt::PadLookDeadzone,
                    Opt::PadGlyphs,
                    Opt::PadMenuNav,
                    Opt::PadKeyboard,
                ]
                .map(HRow::Opt),
            );
            r
        },
    },
    Section {
        name: "Accessibility",
        blurb: "Contrast, motion, text backgrounds and notifications.",
        rows: |_| {
            [
                Opt::HighContrast,
                Opt::MenuBlur,
                Opt::TextBackgroundOpacity,
                Opt::NotificationDisplayTime,
                Opt::ShowSubtitles,
                Opt::ViewBobbing,
                Opt::ChatLineSpacing,
            ]
            .map(HRow::Opt)
            .to_vec()
        },
    },
    Section {
        name: "Language",
        blurb: "The game's language and its font.",
        rows: |_| {
            vec![
                HRow::Page("Language", "", Page::Language),
                HRow::Opt(Opt::ForceUnicode),
                HRow::Opt(Opt::JapaneseGlyphs),
            ]
        },
    },
    Section {
        name: "Chat",
        blurb: "What chat shows and how it looks.",
        rows: |_| {
            [
                Opt::ChatVisibility,
                Opt::ChatColors,
                Opt::ChatLinks,
                Opt::ChatOpacity,
                Opt::ChatScale,
                Opt::ChatWidth,
                Opt::ChatHeightFocused,
                Opt::ChatHeightUnfocused,
                Opt::AutoSuggestions,
                Opt::ReducedDebugInfo,
            ]
            .map(HRow::Opt)
            .to_vec()
        },
    },
    Section {
        name: "Interface",
        blurb: "Menus and resource packs.",
        rows: |_| {
            vec![
                HRow::Style,
                HRow::Page("Resource packs", "Textures, sounds and fonts", Page::Packs),
            ]
        },
    },
    Section {
        name: "Extras",
        blurb: "Optional additions, all off by default: the game stays vanilla until you turn one on.",
        rows: |_| {
            extras::rows()
                .into_iter()
                .map(|r| r.map_or_else(HRow::Header, HRow::Extra))
                .collect()
        },
    },
];

const NAV: u64 = id("options.nav", 0);
const DONE: u64 = id("options.done", 0);
const ROW: u64 = id("options.row", 0);
const STYLE: u64 = id("options.style", 0);
const LEVEL: u64 = id("options.level", 0);
const ROW_H: f32 = 12.0 + CONTROL + 12.0;
const HEADER_H: f32 = 40.0;

/// Whether an option is an on/off switch, and its value.
fn switch_value(opt: Opt, s: &Settings) -> Option<bool> {
    Some(match opt {
        Opt::InvertY => s.invert_y,
        Opt::DiscreteScroll => s.discrete_scroll,
        Opt::ViewBobbing => s.view_bobbing,
        Opt::Vsync => s.vsync,
        Opt::Fullscreen => s.fullscreen,
        Opt::SmoothLighting => s.smooth_lighting,
        Opt::EntityShadows => s.entity_shadows,
        Opt::Vignette => s.vignette,
        Opt::Autosave => s.autosave_indicator,
        Opt::CutoutLeaves => s.cutout_leaves,
        Opt::ImprovedTransparency => s.improved_transparency,
        Opt::AutoJump => s.auto_jump,
        Opt::ChatColors => s.chat_colors,
        Opt::ChatLinks => s.chat_links,
        Opt::AutoSuggestions => s.auto_suggestions,
        Opt::ReducedDebugInfo => s.reduced_debug_info,
        Opt::HighContrast => s.high_contrast,
        Opt::ShowSubtitles => s.show_subtitles,
        Opt::ForceUnicode => s.force_unicode,
        Opt::JapaneseGlyphs => s.japanese_glyphs,
        Opt::PadEnabled => s.controller.enabled,
        Opt::PadVibration => s.controller.vibration,
        Opt::PadInvertX => s.controller.invert_x,
        Opt::PadInvertY => s.controller.invert_y,
        _ => return None,
    })
}
/// The option's caption and value text, from its vanilla label
/// ("Caption: Value") and its caption key.
fn caption_value(opt: Opt, s: &Settings) -> (String, String) {
    let label = opt.label(s).plain();
    let caption = crate::text::Text::translate(opt.caption(), &[]).plain();
    if let Some(rest) = label.strip_prefix(&caption) {
        return (caption, rest.trim_start_matches([':', ' ']).to_owned());
    }
    match label.split_once(": ") {
        Some((c, v)) => (c.to_owned(), v.to_owned()),
        None => (caption, label),
    }
}
/// What a click hit.
enum Target {
    Nav(usize),
    Done,
    Row(usize),
    Slider(Item, f32),
    Style(usize),
    Level(usize),
}
impl OptionsUi {
    /// Lay out (and draw, on a real frame) the Horizon options shell.
    /// Returns the sliders' tracks (option, x, width) for dragging.
    fn horizon_layout(
        &mut self,
        s: &Settings,
        ui: &mut Ui,
        backdrop: f32,
    ) -> Vec<(Item, f32, f32)> {
        let (w, h) = (ui.w, ui.h);
        if self.in_world {
            ui.atmosphere(horizon_theme::VEIL, 1.0);
        } else {
            if backdrop < 1.0 {
                ui.cold_background(1.0 - backdrop);
            }
            ui.atmosphere(horizon_theme::VEIL, 1.0);
        }
        let section = self.section.min(SECTIONS.len() - 1);
        // The sidebar.
        ui.sidebar_frame("Options");
        let names: Vec<&str> = SECTIONS.iter().map(|s| s.name).collect();
        ui.sidebar_list(NAV, 12.0, 72.0, SIDEBAR - 24.0, &names, section);
        let dy = h - 12.0 - CONTROL;
        ui.button(
            DONE,
            [12.0, dy, SIDEBAR - 24.0, CONTROL],
            "Done",
            Button::Secondary,
            true,
        );
        // The page fades in and rises 8 when the section changes.
        let p = ui.tween(id("options.page", section as u64), 1.0, t::move_());
        if self.section_shown != Some(section) {
            self.section_shown = Some(section);
            ui.restart(id("options.page", section as u64), 0.0, 1.0, t::move_());
        }
        let x = SIDEBAR + 48.0;
        let cw = (w - x - 48.0).min(760.0);
        ui.opacity = p;
        ui.offset = (0.0, 8.0 * (1.0 - p));
        let sec = &SECTIONS[section];
        let mut y = 36.0;
        y += ui.page_header(x, y, sec.name, sec.blurb) + 24.0;
        let top = y;
        let rows = (sec.rows)(s);
        let content: f32 = rows
            .iter()
            .map(|r| {
                if matches!(r, HRow::Header(_)) {
                    HEADER_H
                } else {
                    ROW_H
                }
            })
            .sum();
        let bottom = h - 24.0;
        let max = (content - (bottom - top)).max(0.0);
        self.hscroll = self.hscroll.clamp(0.0, max);
        y -= self.hscroll;
        let vstart = ui.batch.vertices.len();
        let hstart = ui.hits.len();
        let tstart = crate::widgets::targets_mark();
        let mut sliders = Vec::new();
        ui.divider(x, y, cw);
        let control_x = x + cw;
        for (i, row) in rows.iter().enumerate() {
            match *row {
                HRow::Header(label) => {
                    ui.caps(x, y + 20.0, color::FAINT, label);
                    y += HEADER_H;
                    ui.divider(x, y, cw);
                    continue;
                }
                HRow::Opt(opt) => {
                    let (caption, value) = caption_value(opt, s);
                    let cy = y + ROW_H / 2.0;
                    if let Some(on) = switch_value(opt, s) {
                        ui.row(x, y, &caption, "", 0.0);
                        ui.switch(ROW + i as u64, control_x, cy, on);
                    } else if let Kind::Slider(v) = opt.kind(s) {
                        let sw = 220.0_f32.min(cw - ROW_LABEL - 200.0).max(80.0);
                        ui.row(x, y, &caption, &value, cw - ROW_LABEL - 16.0 - sw - 24.0);
                        let sx = control_x - sw - 8.0;
                        ui.slider(ROW + i as u64, sx, cy, sw, v);
                        sliders.push((Item::Vanilla(opt), sx, sw));
                    } else {
                        ui.row(x, y, &caption, "", 0.0);
                        let bw = ui.button_width(&value, false).max(96.0);
                        ui.button(
                            ROW + i as u64,
                            [control_x - bw, y + 12.0, bw, CONTROL],
                            &value,
                            Button::Secondary,
                            true,
                        );
                    }
                }
                HRow::Page(label, value, _) => {
                    let value = if value.is_empty() {
                        self.languages
                            .iter()
                            .find(|l| l.code == s.language)
                            .map_or_else(|| s.language.clone(), |l| l.label())
                    } else {
                        value.to_owned()
                    };
                    ui.row(x, y, label, &value, cw - ROW_LABEL - 140.0);
                    let bw = ui.button_width("Change…", false);
                    let open = if label == "Language" {
                        "Change…"
                    } else {
                        "Open…"
                    };
                    let bw = bw.max(ui.button_width(open, false));
                    ui.button(
                        ROW + i as u64,
                        [control_x - bw, y + 12.0, bw, CONTROL],
                        open,
                        Button::Secondary,
                        true,
                    );
                }
                HRow::Level => {
                    ui.row(x, y, "Settings", "", 0.0);
                    let labels = ["Simple", "Advanced"];
                    let probe = ui.segmented(LEVEL, -10_000.0, -10_000.0, &labels, 0);
                    let selected = usize::from(s.horizon.advanced);
                    ui.segmented(LEVEL, control_x - probe, y + 12.0 - 3.0, &labels, selected);
                }
                HRow::Graphics(item, cost, text) => {
                    let (caption, value) = item.caption_value(s);
                    let cost = (cost as u8, cost.label());
                    let cy = y + ROW_H / 2.0;
                    let id = ROW + i as u64;
                    if let Some(on) = item.switch(s) {
                        ui.described_row(x, y, &caption, "", 0.0, cost, text, cw - 60.0);
                        ui.switch(id, control_x, cy, on);
                    } else if let Some(v) = item.slider(s) {
                        let sw = 220.0_f32.min(cw - ROW_LABEL - 200.0).max(80.0);
                        let value_w = cw - ROW_LABEL - 16.0 - sw - 24.0;
                        let text_w = cw - sw - 24.0;
                        ui.described_row(x, y, &caption, &value, value_w, cost, text, text_w);
                        let sx = control_x - sw - 8.0;
                        ui.slider(id, sx, cy, sw, v);
                        sliders.push((item, sx, sw));
                    } else {
                        let bw = ui.button_width(&value, false).max(96.0);
                        let text_w = cw - bw - 24.0;
                        ui.described_row(x, y, &caption, "", 0.0, cost, text, text_w);
                        ui.button(
                            id,
                            [control_x - bw, y + 12.0, bw, CONTROL],
                            &value,
                            Button::Secondary,
                            true,
                        );
                    }
                }
                HRow::Extra(item) => {
                    let cy = y + ROW_H / 2.0;
                    let id = ROW + i as u64;
                    let mut cost = item.cost().map_or((0, ""), |c| (c as u8, c.label()));
                    let (caption, mut text) = (item.caption(), item.description().to_owned());
                    // A vanilla mapping on the same key: both act on it, like
                    // the binds list's duplicates; say which.
                    let conflicts = extras::zoom_key_conflicts(&s.extras, &s.keys);
                    if item == extras::Item::ZoomKey && !conflicts.is_empty() {
                        let names: Vec<String> =
                            conflicts.iter().map(|c| crate::lang::get(c)).collect();
                        cost = (3, "Also bound");
                        text = format!("{} uses this key too; both act on it.", names.join(", "));
                    }
                    let text = text.as_str();
                    if let Some(on) = item.switch(&s.extras) {
                        ui.described_row(x, y, caption, "", 0.0, cost, text, cw - 60.0);
                        ui.switch(id, control_x, cy, on);
                    } else {
                        let value = if item == extras::Item::ZoomKey
                            && self.selected_key == Some(extras::ZOOM_KEY)
                        {
                            "Press a key".to_owned()
                        } else {
                            item.value(&s.extras)
                        };
                        let bw = ui.button_width(&value, false).max(96.0);
                        ui.described_row(x, y, caption, "", 0.0, cost, text, cw - bw - 24.0);
                        ui.button(
                            id,
                            [control_x - bw, y + 12.0, bw, CONTROL],
                            &value,
                            Button::Secondary,
                            true,
                        );
                    }
                }
                HRow::Style => {
                    ui.row(x, y, "Menu style", "", 0.0);
                    let labels = ["Horizon", "Classic"];
                    let probe = ui.segmented(STYLE, -10_000.0, -10_000.0, &labels, 0);
                    let selected = usize::from(t::style() == MenuStyle::Classic);
                    ui.segmented(STYLE, control_x - probe, y + 12.0 - 3.0, &labels, selected);
                }
            }
            y += ROW_H;
            ui.divider(x, y, cw);
        }
        // Clip the rows to the page below the header, like the launcher's scroll view.
        let (ox, oy) = ui.offset;
        let scale = ui.batch.scale;
        let k = ui.density / scale;
        let (y0, y1) = ((top - 1.0 + oy) * k, (bottom + oy) * k);
        ui.batch.clip_since(vstart, 0.0, y0, ui.batch.width, y1);
        if ui.is_real() {
            crate::widgets::clip_targets(tstart, y0, y1);
        }
        let (cy0, cy1) = (top - 1.0 + oy, bottom + oy);
        let kept: Vec<_> = ui
            .hits
            .drain(hstart..)
            .filter(|h| h.rect[1] >= cy0 && h.rect[3] <= cy1)
            .collect();
        ui.hits.extend(kept);
        if max > 0.0 {
            // A thin scroll indicator.
            let track = bottom - top;
            let bar = (track * track / (track + max)).max(24.0);
            let by = top + (track - bar) * (self.hscroll / max);
            ui.rounded(
                x + cw + 16.0,
                by,
                3.0,
                bar,
                1.5,
                color::with_alpha(color::MUTED, 0.35),
            );
        }
        let _ = ox;
        ui.opacity = 1.0;
        ui.offset = (0.0, 0.0);
        sliders
    }
    pub(crate) fn draw_horizon(
        &mut self,
        s: &Settings,
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        cursor: (f32, f32),
    ) {
        let mut ui = Ui::new(atlas, batch, cursor);
        let backdrop = t::backdrop();
        self.horizon_layout(s, &mut ui, backdrop);
        ui.finish();
    }
    fn horizon_target(&mut self, s: &Settings, atlas: &HudAtlas, x: f32, y: f32) -> Option<Target> {
        let mut batch = t::scratch_batch();
        let mut ui = Ui::dry(atlas, &mut batch, (x, y));
        let sliders = self.horizon_layout(s, &mut ui, 1.0);
        let hit = ui.hit((x, y))?;
        let section = self.section.min(SECTIONS.len() - 1);
        if (NAV..NAV + SECTIONS.len() as u64).contains(&hit) {
            return Some(Target::Nav((hit - NAV) as usize));
        }
        if hit == DONE {
            return Some(Target::Done);
        }
        if hit == STYLE || hit == STYLE + 1 {
            return Some(Target::Style((hit - STYLE) as usize));
        }
        if hit == LEVEL || hit == LEVEL + 1 {
            return Some(Target::Level((hit - LEVEL) as usize));
        }
        let i = hit.checked_sub(ROW)? as usize;
        let rows = (SECTIONS[section].rows)(s);
        let item = match rows.get(i) {
            Some(HRow::Opt(opt)) => Some(Item::Vanilla(*opt)),
            Some(HRow::Graphics(item, ..)) => Some(*item),
            _ => None,
        };
        if let Some(item) = item
            && let Some(&(_, sx, sw)) = sliders.iter().find(|(o, ..)| *o == item)
        {
            let v = t::slider_value(&ui, sx, sw, x);
            return Some(Target::Slider(item, v));
        }
        (i < rows.len()).then_some(Target::Row(i))
    }
    /// A click (or a drag with the button held) on the Horizon shell.
    pub(crate) fn click_horizon(
        &mut self,
        s: &mut Settings,
        atlas: &HudAtlas,
        x: f32,
        y: f32,
        framebuffer: [i32; 2],
        drag: bool,
        right: bool,
    ) -> Vec<Event> {
        let mut events = Vec::new();
        if drag {
            if let Some(item) = self.hdragging {
                let mut batch = t::scratch_batch();
                let mut ui = Ui::dry(atlas, &mut batch, (x, y));
                let sliders = self.horizon_layout(s, &mut ui, 1.0);
                if let Some(&(_, sx, sw)) = sliders.iter().find(|(o, ..)| *o == item) {
                    set_item_slider(item, s, t::slider_value(&ui, sx, sw, x));
                    events.push(Event::Changed);
                }
            }
            return events;
        }
        // A click anywhere stops waiting for the zoom key.
        if self.selected_key == Some(extras::ZOOM_KEY) {
            self.selected_key = None;
        }
        match self.horizon_target(s, atlas, x, y) {
            Some(Target::Nav(i)) => {
                self.section = i;
                self.hscroll = 0.0;
            }
            Some(Target::Done) => events.extend(self.back(s)),
            Some(Target::Style(i)) => {
                t::set_style(if i == 1 {
                    MenuStyle::Classic
                } else {
                    MenuStyle::Horizon
                });
                if let Ok(dir) = std::env::current_dir() {
                    t::save_preferences(&dir);
                }
            }
            Some(Target::Level(i)) => {
                s.horizon.advanced = i == 1;
                self.hscroll = 0.0;
                events.push(Event::Changed);
            }
            Some(Target::Slider(item, v)) => {
                self.hdragging = Some(item);
                set_item_slider(item, s, v);
                events.push(Event::Changed);
            }
            Some(Target::Row(i)) => {
                let section = self.section.min(SECTIONS.len() - 1);
                match (SECTIONS[section].rows)(s).get(i) {
                    Some(HRow::Opt(opt)) => {
                        opt.cycle(s, right, framebuffer);
                        if *opt == Opt::Preset {
                            apply_preset(s);
                        }
                        events.push(Event::Changed);
                    }
                    Some(HRow::Graphics(item, ..)) => {
                        item.cycle(s, right, framebuffer);
                        events.push(Event::Changed);
                    }
                    Some(HRow::Page(_, _, page)) => self.open_page(*page),
                    Some(HRow::Extra(extras::Item::ZoomKey)) => {
                        self.selected_key = Some(extras::ZOOM_KEY);
                    }
                    Some(HRow::Extra(item)) => {
                        item.cycle(&mut s.extras, right);
                        events.push(Event::Changed);
                    }
                    _ => {}
                }
            }
            None => {}
        }
        events
    }
    pub(crate) fn scroll_horizon(&mut self, amount: f32) {
        self.hscroll -= amount * 40.0;
    }
}

/// A slider moved: vanilla options keep their preset handling, Horizon's own
/// settings go through `graphics_ui`.
fn set_item_slider(item: Item, s: &mut Settings, v: f32) {
    match item {
        Item::Vanilla(opt) => {
            opt.set_slider(s, v);
            if opt == Opt::Preset {
                apply_preset(s);
            }
        }
        Item::Horizon(_) => item.set_slider(s, v),
    }
}
