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
    options_ui::{self, Event, Kind, Opt, OptionsUi, Page, apply_preset},
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
    /// Installed mod `i` (`settings.mods.installed`) and its switch.
    Mod(usize),
    /// Opens the directory where Horizon mods are installed.
    OpenModFolder,
    /// `shaderpacks/`'s path and a button opening it.
    ShaderPackFolder,
    /// Shader pack `i` of `OptionsUi::shader_packs` and its switch.
    ShaderPack(usize),
}

/// The sidebar section listing the shader packs.
pub const SHADER_PACKS: &str = "Shader Packs";
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
                    Opt::PadAdaptiveTriggers,
                    Opt::PadLightbar,
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
            let mut rows = vec![HRow::Page(
                "Controls",
                "Key binds, mouse and controller",
                Page::Controls,
            )];
            rows.extend(
                options_ui::ACCESSIBILITY_OPTIONS
                    .iter()
                    .copied()
                    .filter(|opt| opt.available())
                    .map(HRow::Opt),
            );
            rows
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
        name: SHADER_PACKS,
        blurb: "Packs that redraw the world. Off is the vanilla look.",
        // The packs follow, from the folder (`horizon_rows`).
        rows: |_| vec![HRow::ShaderPackFolder],
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
        // A switch's own settings nest under it and show while it is on, so
        // the page stays one row per extra until one is turned on.
        rows: |s| {
            extras::rows()
                .into_iter()
                .filter(|r| {
                    r.map_or(true, |item| {
                        item.parent()
                            .is_none_or(|parent| parent.switch(&s.extras) == Some(true))
                    })
                })
                .map(|r| r.map_or_else(HRow::Header, HRow::Extra))
                .collect()
        },
    },
    Section {
        name: "Mods",
        blurb: "Mods made for Horizon, from the horizon-mods folder. Each runs in a sandbox and stays off until you turn it on; check what it may do first.",
        rows: |s| {
            let mut rows = vec![HRow::OpenModFolder];
            if s.mods.installed.is_empty() {
                rows.push(HRow::Header(
                    "No mods: put .hmod files in the horizon-mods folder",
                ));
            } else {
                rows.extend((0..s.mods.installed.len()).map(HRow::Mod));
            }
            rows
        },
    },
];

const NAV: u64 = id("options.nav", 0);
const DONE: u64 = id("options.done", 0);
const ROW: u64 = id("options.row", 0);
const STYLE: u64 = id("options.style", 0);
const LEVEL: u64 = id("options.level", 0);
const SEARCH: u64 = id("options.search", 0);
const ROW_H: f32 = 12.0 + CONTROL + 12.0;
const HEADER_H: f32 = 40.0;
pub(crate) fn searchable_section(section: usize) -> bool {
    matches!(
        SECTIONS.get(section).map(|s| s.name),
        Some("Video" | "Controls")
    )
}

/// Whether an option is an on/off switch, and its value.
fn switch_value(opt: Opt, s: &Settings) -> Option<bool> {
    Some(match opt {
        Opt::HideLightningFlashes => s.file.bool("hideLightningFlashes"),
        Opt::HideSplashTexts => s.file.bool("hideSplashTexts"),
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
        Opt::PadAdaptiveTriggers => s.controller.adaptive_triggers,
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
    Search,
    Row(usize),
    Slider(Item, f32),
    Style(usize),
    Level(usize),
}
struct HorizonScrollbar {
    x: f32,
    top: f32,
    bottom: f32,
    max: f32,
    thumb: f32,
}
impl HorizonScrollbar {
    fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + 6.0 && y >= self.top && y < self.bottom
    }
}
struct HorizonLayout {
    sliders: Vec<(Item, f32, f32)>,
    scrollbar: Option<HorizonScrollbar>,
}
fn horizon_batch(framebuffer: [i32; 2]) -> HudBatch {
    HudBatch::new([framebuffer[0].max(1) as u32, framebuffer[1].max(1) as u32])
}
impl OptionsUi {
    fn horizon_rows(&self, s: &Settings, section: usize) -> Vec<HRow> {
        let mut rows = (SECTIONS[section].rows)(s);
        if SECTIONS[section].name == SHADER_PACKS {
            if self.shader_packs.packs.is_empty() {
                rows.push(HRow::Header(
                    "No shader packs: put a pack's folder or .zip in this folder",
                ));
            }
            rows.extend((0..self.shader_packs.packs.len()).map(HRow::ShaderPack));
        }
        let query = self.settings_search.value.to_lowercase();
        if query.is_empty() || !searchable_section(section) {
            return rows;
        }
        rows.into_iter()
            .filter(|row| match row {
                HRow::Opt(opt) => caption_value(*opt, s).0.to_lowercase().contains(&query),
                HRow::Graphics(item, ..) => item.caption_value(s).0.to_lowercase().contains(&query),
                HRow::Page(label, ..) => label.to_lowercase().contains(&query),
                HRow::Level => true,
                _ => false,
            })
            .collect()
    }
    /// Lay out (and draw, on a real frame) the Horizon options shell.
    /// Returns the sliders' tracks and the scroll area for pointer input.
    fn horizon_layout(&mut self, s: &Settings, ui: &mut Ui, backdrop: f32) -> HorizonLayout {
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
        if searchable_section(section) {
            ui.text_field(
                SEARCH,
                [x + cw - 240.0, y, 240.0, CONTROL],
                &self.settings_search.value,
                self.settings_search.focused,
            );
            y += CONTROL + 12.0;
        }
        let top = y;
        let rows = self.horizon_rows(s, section);
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
                    if !opt.available() {
                        ui.row(x, y, &caption, "", 0.0);
                        let label = "Unavailable";
                        let bw = ui.button_width(label, false).max(96.0);
                        ui.button(
                            ROW + i as u64,
                            [control_x - bw, y + 12.0, bw, CONTROL],
                            label,
                            Button::Secondary,
                            false,
                        );
                    } else if let Some(on) = switch_value(opt, s) {
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
                HRow::Mod(m) => {
                    let installed = &s.mods.installed[m];
                    let cost = match (&installed.problem, installed.dev) {
                        (Some(_), _) => (3, "Stopped"),
                        (None, true) => (1, "Dev"),
                        (None, false) => (0, ""),
                    };
                    let text = installed
                        .problem
                        .as_deref()
                        .unwrap_or(&installed.description);
                    ui.described_row(x, y, &installed.title, "", 0.0, cost, text, cw - 60.0);
                    ui.switch(ROW + i as u64, control_x, y + ROW_H / 2.0, s.mods.is_on(m));
                }
                HRow::ShaderPackFolder => {
                    let label = "Open Folder";
                    let bw = ui.button_width(label, false).max(96.0);
                    let dir = self.shader_packs.dir.display().to_string();
                    ui.described_row(x, y, "Folder", "", 0.0, (0, ""), &dir, cw - bw - 24.0);
                    ui.button(
                        ROW + i as u64,
                        [control_x - bw, y + 12.0, bw, CONTROL],
                        label,
                        Button::Secondary,
                        true,
                    );
                }
                HRow::ShaderPack(p) => {
                    let (id, info) = &self.shader_packs.packs[p];
                    let error = match info {
                        Err(e) => Some(e.as_str()),
                        Ok(_) => self
                            .shader_pack_error
                            .as_ref()
                            .filter(|(failed, _)| failed == id)
                            .map(|(_, e)| e.as_str()),
                    };
                    let (cost, text) = match (error, info) {
                        (Some(e), _) => ((3, "Failed"), e),
                        (None, Ok(info)) => ((0, ""), info.description.as_str()),
                        (None, Err(_)) => unreachable!(),
                    };
                    ui.described_row(x, y, id, "", 0.0, cost, text, cw - 60.0);
                    let on = s.horizon.shader_pack == *id;
                    ui.switch(ROW + i as u64, control_x, y + ROW_H / 2.0, on);
                }
                HRow::OpenModFolder => {
                    ui.row(x, y, "Mod folder", "", 0.0);
                    let label = "Open Folder";
                    let bw = ui.button_width(label, false).max(96.0);
                    ui.button(
                        ROW + i as u64,
                        [control_x - bw, y + 12.0, bw, CONTROL],
                        label,
                        Button::Secondary,
                        true,
                    );
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
                        let value =
                            if item.key_name().is_some() && self.selected_key == item.key_name() {
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
        let mut scrollbar = None;
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
            scrollbar = Some(HorizonScrollbar {
                x: x + cw + 14.0,
                top: top + oy,
                bottom: bottom + oy,
                max,
                thumb: bar,
            });
        }
        let _ = ox;
        ui.opacity = 1.0;
        ui.offset = (0.0, 0.0);
        HorizonLayout { sliders, scrollbar }
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
    fn horizon_target(
        &mut self,
        s: &Settings,
        atlas: &HudAtlas,
        x: f32,
        y: f32,
        framebuffer: [i32; 2],
    ) -> (Option<Target>, Option<HorizonScrollbar>, (f32, f32)) {
        let mut batch = horizon_batch(framebuffer);
        let mut ui = Ui::dry(atlas, &mut batch, (x, y));
        let layout = self.horizon_layout(s, &mut ui, 1.0);
        let scrollbar = layout.scrollbar;
        // The UI layer transforms raw GUI-pixel input into its own dp.
        let cursor = ui.cursor;
        let Some(hit) = ui.hit((x, y)) else {
            return (None, scrollbar, cursor);
        };
        if hit == SEARCH {
            return (Some(Target::Search), scrollbar, cursor);
        }
        let section = self.section.min(SECTIONS.len() - 1);
        if (NAV..NAV + SECTIONS.len() as u64).contains(&hit) {
            return (Some(Target::Nav((hit - NAV) as usize)), scrollbar, cursor);
        }
        if hit == DONE {
            return (Some(Target::Done), scrollbar, cursor);
        }
        if hit == STYLE || hit == STYLE + 1 {
            return (
                Some(Target::Style((hit - STYLE) as usize)),
                scrollbar,
                cursor,
            );
        }
        if hit == LEVEL || hit == LEVEL + 1 {
            return (
                Some(Target::Level((hit - LEVEL) as usize)),
                scrollbar,
                cursor,
            );
        }
        let Some(i) = hit.checked_sub(ROW).map(|i| i as usize) else {
            return (None, scrollbar, cursor);
        };
        let rows = self.horizon_rows(s, section);
        let item = match rows.get(i) {
            Some(HRow::Opt(opt)) => Some(Item::Vanilla(*opt)),
            Some(HRow::Graphics(item, ..)) => Some(*item),
            _ => None,
        };
        if let Some(item) = item
            && let Some(&(_, sx, sw)) = layout.sliders.iter().find(|(o, ..)| *o == item)
        {
            let v = t::slider_value(&ui, sx, sw, x);
            return (Some(Target::Slider(item, v)), scrollbar, cursor);
        }
        (
            (i < rows.len()).then_some(Target::Row(i)),
            scrollbar,
            cursor,
        )
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
            let mut batch = horizon_batch(framebuffer);
            let mut ui = Ui::dry(atlas, &mut batch, (x, y));
            let layout = self.horizon_layout(s, &mut ui, 1.0);
            // Scrollbar geometry and stored drag positions are in Horizon dp.
            let pointer_y = ui.cursor.1;
            if let Some(previous_y) = self.scrollbar_drag_y {
                if let Some(scrollbar) = layout.scrollbar {
                    // AbstractScrollArea.mouseDragged (inherited by AbstractSelectionList)
                    // moves by the pointer delta and sends out-of-range drags to an end.
                    let track = scrollbar.bottom - scrollbar.top;
                    self.hscroll = if pointer_y < scrollbar.top {
                        0.0
                    } else if pointer_y > scrollbar.bottom {
                        scrollbar.max
                    } else {
                        self.hscroll
                            + (pointer_y - previous_y) * scrollbar.max
                                / (track - scrollbar.thumb).max(1.0)
                    };
                    self.hscroll = self.hscroll.clamp(0.0, scrollbar.max);
                    self.scrollbar_drag_y = Some(pointer_y);
                } else {
                    self.scrollbar_drag_y = None;
                }
                return events;
            }
            if let Some(item) = self.hdragging
                && let Some(&(_, sx, sw)) = layout.sliders.iter().find(|(o, ..)| *o == item)
            {
                set_item_slider(item, s, t::slider_value(&ui, sx, sw, x));
                events.push(Event::Changed);
            }
            return events;
        }
        // Rebinding consumes the click before sidebar or widget actions.
        if matches!(self.selected_key, Some(extras::ZOOM_KEY | extras::SORT_KEY)) {
            return self.bind_mouse(
                s,
                if right {
                    winit::event::MouseButton::Right
                } else {
                    winit::event::MouseButton::Left
                },
            );
        }
        let (target, scrollbar, cursor) = self.horizon_target(s, atlas, x, y, framebuffer);
        self.settings_search.focused = false;
        // AbstractScrollArea.updateScrolling grabs anywhere in the scrollbar column.
        if !right
            && let Some(scrollbar) = scrollbar
            && scrollbar.contains(cursor.0, cursor.1)
        {
            self.scrollbar_drag_y = Some(cursor.1);
            return events;
        }
        match target {
            Some(Target::Nav(i)) => self.select_section(i),
            Some(Target::Search) => {
                self.settings_search.focused = true;
                self.settings_search.move_cursor_to_end(false);
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
                match self.horizon_rows(s, section).get(i) {
                    Some(HRow::Opt(opt)) if opt.available() => {
                        opt.cycle(s, right, framebuffer);
                        if *opt == Opt::Preset {
                            apply_preset(s);
                        }
                        events.push(Event::Changed);
                    }
                    Some(HRow::Graphics(Item::Horizon(graphics_ui::H::ShaderPack), ..)) => {
                        events.push(Event::ButtonClick);
                        self.open_section(SHADER_PACKS);
                    }
                    Some(HRow::Graphics(item, ..)) => {
                        item.cycle(s, right, framebuffer);
                        events.push(Event::Changed);
                    }
                    Some(HRow::Page(_, _, page)) => self.open_page(*page),
                    Some(HRow::Extra(item)) if item.key_name().is_some() => {
                        self.selected_key = item.key_name();
                    }
                    Some(HRow::Extra(item)) => {
                        item.cycle(&mut s.extras, right);
                        events.push(Event::Changed);
                    }
                    Some(HRow::Mod(m)) => {
                        s.mods.toggle(*m);
                        events.push(Event::Changed);
                    }
                    Some(HRow::OpenModFolder) if !right => {
                        events.push(Event::ButtonClick);
                        events.push(Event::OpenModFolder);
                    }
                    Some(HRow::ShaderPackFolder) if !right => {
                        events.push(Event::ButtonClick);
                        events.push(Event::OpenShaderPackFolder);
                    }
                    Some(HRow::ShaderPack(p)) => {
                        self.toggle_shader_pack(s, *p);
                        events.push(Event::Changed);
                    }
                    _ => {}
                }
            }
            None => {}
        }
        events
    }
    fn select_section(&mut self, i: usize) {
        if searchable_section(self.section) || searchable_section(i) {
            self.settings_search.set_value("");
        }
        self.section = i;
        self.hscroll = 0.0;
    }
    /// Show the sidebar section called `name`.
    fn open_section(&mut self, name: &str) {
        if let Some(i) = SECTIONS.iter().position(|s| s.name == name) {
            self.select_section(i);
        }
    }
    pub(crate) fn scroll_horizon(&mut self, amount: f32) {
        self.hscroll -= amount * 40.0;
    }
}

/// A slider moved: vanilla options keep their preset handling, Horizon's own
/// settings go through `graphics_ui`.
fn set_item_slider(item: Item, s: &mut Settings, v: f32) {
    match item {
        Item::Vanilla(opt) => options_ui::set_slider(opt, s, v),
        Item::Horizon(_) => item.set_slider(s, v),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn atlas() -> HudAtlas {
        HudAtlas::build(std::path::Path::new("/nonexistent/horizon-test-assets")).unwrap()
    }

    fn section_ui(name: &str) -> OptionsUi {
        let mut ui = OptionsUi::default();
        ui.section = SECTIONS
            .iter()
            .position(|section| section.name == name)
            .unwrap();
        ui
    }

    fn extras_ui() -> OptionsUi {
        section_ui("Extras")
    }

    fn scrollbar(
        ui: &mut OptionsUi,
        settings: &Settings,
        atlas: &HudAtlas,
        framebuffer: [i32; 2],
    ) -> HorizonScrollbar {
        let mut batch = horizon_batch(framebuffer);
        let mut draw = Ui::dry(atlas, &mut batch, (0.0, 0.0));
        ui.horizon_layout(settings, &mut draw, 1.0)
            .scrollbar
            .unwrap()
    }

    fn gui_pointer(atlas: &HudAtlas, framebuffer: [i32; 2], point: (f32, f32)) -> (f32, f32) {
        let mut batch = horizon_batch(framebuffer);
        let ui = Ui::dry(atlas, &mut batch, (0.0, 0.0));
        let gui_per_dp = ui.density / ui.batch.scale;
        (point.0 * gui_per_dp, point.1 * gui_per_dp)
    }

    #[test]
    fn extras_settings_nest_under_their_switch() {
        let mut s = Settings::default();
        let section = SECTIONS.iter().find(|s| s.name == "Extras").unwrap();
        let items = |s: &Settings| -> Vec<extras::Item> {
            (section.rows)(s)
                .iter()
                .filter_map(|row| match row {
                    HRow::Extra(item) => Some(*item),
                    _ => None,
                })
                .collect()
        };
        // Everything off: one row per extra.
        assert!(items(&s).iter().all(|item| item.parent().is_none()));
        s.extras.zoom = true;
        let zoom_on = items(&s);
        assert!(zoom_on.contains(&extras::Item::ZoomKey));
        assert!(zoom_on.contains(&extras::Item::ZoomSensitivity));
        assert!(!zoom_on.contains(&extras::Item::MinimapSize));
        // Every setting shows once its switch is on.
        for item in extras::rows().into_iter().flatten() {
            if let Some(on) = item.switch(&s.extras)
                && item.parent().is_none()
                && !on
            {
                item.cycle(&mut s.extras, false);
            }
        }
        assert_eq!(
            items(&s).len(),
            extras::rows().into_iter().flatten().count()
        );
    }

    #[test]
    fn accessibility_section_only_shows_available_options() {
        let s = Settings::default();
        let section = SECTIONS.iter().find(|s| s.name == "Accessibility").unwrap();
        let rows = (section.rows)(&s);
        let options: Vec<_> = rows
            .iter()
            .filter_map(|row| match row {
                HRow::Opt(opt) => Some(*opt),
                _ => None,
            })
            .collect();
        let expected: Vec<_> = options_ui::ACCESSIBILITY_OPTIONS
            .iter()
            .copied()
            .filter(|opt| opt.available())
            .collect();
        assert_eq!(options, expected);
        assert!(options.iter().all(|opt| opt.available()));
        assert!(
            rows.iter()
                .any(|r| matches!(r, HRow::Page(_, _, Page::Controls)))
        );
    }

    #[test]
    fn horizon_scrollbar_track_drag_is_bounded_and_stops_on_release() {
        let atlas = atlas();
        let framebuffer = [2560, 1440];
        let mut settings = Settings::default();
        let mut ui = section_ui("Controls");
        let bar = scrollbar(&mut ui, &settings, &atlas, framebuffer);
        let max = bar.max;
        let track = bar.bottom - bar.top;
        assert!(max > 0.0);

        // Start on the track below the thumb, then move horizontally away;
        // the held scrollbar keeps following the pointer.
        let x = bar.x + 2.0;
        let y = bar.top + bar.thumb + (track - bar.thumb) / 2.0;
        assert!(y < bar.bottom);
        let (click_x, click_y) = gui_pointer(&atlas, framebuffer, (x, y));
        assert!(
            ui.click_horizon(
                &mut settings,
                &atlas,
                click_x,
                click_y,
                framebuffer,
                false,
                false,
            )
            .is_empty()
        );
        assert_eq!(ui.hscroll, 0.0);
        let delta = 5.0;
        let (drag_x, drag_y) = gui_pointer(&atlas, framebuffer, (x - 100.0, y + delta));
        assert!(
            ui.click_horizon(
                &mut settings,
                &atlas,
                drag_x,
                drag_y,
                framebuffer,
                true,
                false,
            )
            .is_empty()
        );
        assert!((ui.hscroll - delta * max / (track - bar.thumb)).abs() < 0.001);

        let (below_x, below_y) = gui_pointer(&atlas, framebuffer, (x, bar.bottom + 100.0));
        assert!(
            ui.click_horizon(
                &mut settings,
                &atlas,
                below_x,
                below_y,
                framebuffer,
                true,
                false,
            )
            .is_empty()
        );
        assert_eq!(ui.hscroll, max);
        let (above_x, above_y) = gui_pointer(&atlas, framebuffer, (x, bar.top - 100.0));
        assert!(
            ui.click_horizon(
                &mut settings,
                &atlas,
                above_x,
                above_y,
                framebuffer,
                true,
                false,
            )
            .is_empty()
        );
        assert_eq!(ui.hscroll, 0.0);

        assert!(ui.release().is_empty());
        let (below_x, below_y) = gui_pointer(&atlas, framebuffer, (x, bar.bottom + 100.0));
        assert!(
            ui.click_horizon(
                &mut settings,
                &atlas,
                below_x,
                below_y,
                framebuffer,
                true,
                false,
            )
            .is_empty()
        );
        assert_eq!(ui.hscroll, 0.0);
        assert_eq!(settings.keys, Settings::default().keys);
    }

    #[test]
    fn horizon_key_rebinding_takes_precedence_over_scrollbar_press() {
        let atlas = atlas();
        let framebuffer = [2560, 1440];
        let mut settings = Settings::default();
        let mut ui = extras_ui();
        let bar = scrollbar(&mut ui, &settings, &atlas, framebuffer);
        let dp_y = bar.top + bar.thumb + (bar.bottom - bar.top - bar.thumb) / 2.0;
        ui.selected_key = Some(extras::SORT_KEY);
        let (x, y) = gui_pointer(&atlas, framebuffer, (bar.x + 2.0, dp_y));

        assert_eq!(
            ui.click_horizon(&mut settings, &atlas, x, y, framebuffer, false, false,),
            vec![Event::Changed]
        );
        assert_eq!(settings.extras.sort_key, "key.mouse.left");
        assert_eq!(ui.selected_key, None);
        assert_eq!(ui.hscroll, 0.0);

        let (x, drag_y) = gui_pointer(&atlas, framebuffer, (bar.x + 2.0, dp_y + 10.0));
        ui.click_horizon(&mut settings, &atlas, x, drag_y, framebuffer, true, false);
        assert_eq!(ui.hscroll, 0.0);
    }

    /// The Shader Packs section: the folder's path and Open Folder, a switch
    /// per pack; the Video section's Shader Pack row leads here (#941, #925).
    #[test]
    fn shader_packs_section_opens_the_folder_and_picks_a_pack() {
        let atlas = atlas();
        let framebuffer = [2560, 1440];
        let dir =
            std::env::temp_dir().join(format!("horizon-shader-section-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut options = section_ui(SHADER_PACKS);
        let mut settings = Settings::default();
        options.shader_packs.poll(dir.clone());
        assert!(matches!(
            options.horizon_rows(&settings, options.section)[..],
            [HRow::ShaderPackFolder, HRow::Header(_)]
        ));
        std::fs::write(dir.join("Pack.zip"), b"").unwrap();
        options.shader_packs = Default::default();
        options.shader_packs.poll(dir.clone());
        // Click the control with hit id `id`, scrolling until it shows.
        let click = |options: &mut OptionsUi, settings: &mut Settings, id: u64| {
            for _ in 0..40 {
                let mut batch = horizon_batch(framebuffer);
                let mut ui = Ui::dry(&atlas, &mut batch, (0.0, 0.0));
                options.horizon_layout(settings, &mut ui, 1.0);
                if let Some(hit) = ui.hits.iter().find(|hit| hit.id == id) {
                    let r = hit.rect;
                    let scale = ui.density / ui.batch.scale;
                    let at = ((r[0] + r[2]) * 0.5 * scale, (r[1] + r[3]) * 0.5 * scale);
                    drop(ui);
                    return options.click_horizon(
                        settings,
                        &atlas,
                        at.0,
                        at.1,
                        framebuffer,
                        false,
                        false,
                    );
                }
                drop(ui);
                options.hscroll += 200.0;
            }
            panic!("no control {id}");
        };
        assert_eq!(
            click(&mut options, &mut settings, ROW),
            [Event::ButtonClick, Event::OpenShaderPackFolder]
        );
        assert_eq!(
            click(&mut options, &mut settings, ROW + 1),
            [Event::Changed]
        );
        assert_eq!(settings.horizon.shader_pack, "Pack.zip");
        click(&mut options, &mut settings, ROW + 1);
        assert_eq!(settings.horizon.shader_pack, "");

        let mut options = section_ui("Video");
        settings.horizon.advanced = true;
        let video = SECTIONS.iter().position(|s| s.name == "Video").unwrap();
        let row = options
            .horizon_rows(&settings, video)
            .iter()
            .position(|r| {
                matches!(
                    r,
                    HRow::Graphics(Item::Horizon(graphics_ui::H::ShaderPack), ..)
                )
            })
            .unwrap();
        assert_eq!(
            click(&mut options, &mut settings, ROW + row as u64),
            [Event::ButtonClick]
        );
        assert_eq!(SECTIONS[options.section].name, SHADER_PACKS);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mods_section_opens_folder_from_its_button() {
        let atlas = atlas();
        let framebuffer = [2560, 1440];
        let mut options = section_ui("Mods");
        let mut settings = Settings::default();
        let mut batch = horizon_batch(framebuffer);
        let mut ui = Ui::dry(&atlas, &mut batch, (0.0, 0.0));
        options.horizon_layout(&settings, &mut ui, 1.0);
        let rect = ui
            .hits
            .iter()
            .find(|hit| hit.id == ROW)
            .expect("Mods section has an Open Folder button")
            .rect;
        let scale = ui.density / ui.batch.scale;
        let cursor = (
            (rect[0] + rect[2]) * 0.5 * scale,
            (rect[1] + rect[3]) * 0.5 * scale,
        );
        drop(ui);

        assert_eq!(
            options.click_horizon(
                &mut settings,
                &atlas,
                cursor.0,
                cursor.1,
                framebuffer,
                false,
                false
            ),
            vec![Event::ButtonClick, Event::OpenModFolder]
        );
        assert!(
            options
                .click_horizon(
                    &mut settings,
                    &atlas,
                    cursor.0,
                    cursor.1,
                    framebuffer,
                    false,
                    true
                )
                .is_empty()
        );
    }
}
