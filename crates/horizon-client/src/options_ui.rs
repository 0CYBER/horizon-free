//! The options screens: OptionsScreen, Video Settings, Music & Sounds,
//! Controls, Key Binds, Mouse Settings, Chat Settings, Accessibility,
//! Language and Resource Packs, laid out like vanilla's HeaderAndFooter /
//! OptionsList screens and drawn with the shared widgets.
use crate::{
    hud::{HudAtlas, HudBatch, rgba},
    keybinds::{self, CATEGORIES, KeyBinds, MAPPINGS},
    lang::{self, LanguageInfo},
    settings::{SOUND_SOURCES, Settings},
    text::{self, Style, Text},
    ui,
};

/// One option instance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Opt {
    Fov,
    RenderDistance,
    SimulationDistance,
    Sensitivity,
    WheelSensitivity,
    InvertY,
    DiscreteScroll,
    Volume(usize),
    GuiScale,
    ViewBobbing,
    Gamma,
    MaxFps,
    Vsync,
    Fullscreen,
    SmoothLighting,
    Clouds,
    CloudRange,
    Particles,
    EntityDistance,
    EntityShadows,
    BiomeBlend,
    Preset,
    Mipmaps,
    WeatherRadius,
    ChunkFade,
    Vignette,
    Autosave,
    CutoutLeaves,
    ImprovedTransparency,
    AttackIndicator,
    InactivityFps,
    ChunkBuilder,
    MenuBlur,
    ToggleCrouch,
    ToggleSprint,
    AutoJump,
    ChatVisibility,
    ChatColors,
    ChatLinks,
    ChatOpacity,
    TextBackgroundOpacity,
    ChatScale,
    ChatWidth,
    ChatHeightFocused,
    ChatHeightUnfocused,
    ChatLineSpacing,
    AutoSuggestions,
    ReducedDebugInfo,
    HighContrast,
    ForceUnicode,
    JapaneseGlyphs,
}
fn tr(key: &str) -> Text {
    Text::translate(key, &[])
}
fn generic(caption: &str, value: Text) -> Text {
    Text::translate("options.generic_value", &[tr(caption), value])
}
fn percent(caption: &str, value: f32) -> Text {
    Text::translate(
        "options.percent_value",
        &[
            tr(caption),
            Text::literal(((f64::from(value) * 100.0) as i32).to_string()),
        ],
    )
}
fn on_off(v: bool) -> Text {
    tr(if v { "options.on" } else { "options.off" })
}
fn chat_px(caption: &str, px: f32) -> Text {
    Text::translate(
        "options.pixel_value",
        &[tr(caption), Text::literal((px as i32).to_string())],
    )
}
enum Kind {
    /// Slider position 0..1.
    Slider(f32),
    Cycle,
}
impl Opt {
    fn caption(self) -> &'static str {
        match self {
            Opt::Fov => "options.fov",
            Opt::RenderDistance => "options.renderDistance",
            Opt::SimulationDistance => "options.simulationDistance",
            Opt::Sensitivity => "options.sensitivity",
            Opt::WheelSensitivity => "options.mouseWheelSensitivity",
            Opt::InvertY => "options.invertMouseY",
            Opt::DiscreteScroll => "options.discrete_mouse_scroll",
            Opt::Volume(_) => "soundCategory.master",
            Opt::GuiScale => "options.guiScale",
            Opt::ViewBobbing => "options.viewBobbing",
            Opt::Gamma => "options.gamma",
            Opt::MaxFps => "options.framerateLimit",
            Opt::Vsync => "options.vsync",
            Opt::Fullscreen => "options.fullscreen",
            Opt::SmoothLighting => "options.ao",
            Opt::Clouds => "options.renderClouds",
            Opt::CloudRange => "options.renderCloudsDistance",
            Opt::Particles => "options.particles",
            Opt::EntityDistance => "options.entityDistanceScaling",
            Opt::EntityShadows => "options.entityShadows",
            Opt::BiomeBlend => "options.biomeBlendRadius",
            Opt::Preset => "options.graphics.preset",
            Opt::Mipmaps => "options.mipmapLevels",
            Opt::WeatherRadius => "options.weatherRadius",
            Opt::ChunkFade => "options.chunkFade",
            Opt::Vignette => "options.vignette",
            Opt::Autosave => "options.autosaveIndicator",
            Opt::CutoutLeaves => "options.cutoutLeaves",
            Opt::ImprovedTransparency => "options.improvedTransparency",
            Opt::AttackIndicator => "options.attackIndicator",
            Opt::InactivityFps => "options.inactivityFpsLimit",
            Opt::ChunkBuilder => "options.prioritizeChunkUpdates",
            Opt::MenuBlur => "options.accessibility.menu_background_blurriness",
            Opt::ToggleCrouch => "key.sneak",
            Opt::ToggleSprint => "key.sprint",
            Opt::AutoJump => "options.autoJump",
            Opt::ChatVisibility => "options.chat.visibility",
            Opt::ChatColors => "options.chat.color",
            Opt::ChatLinks => "options.chat.links",
            Opt::ChatOpacity => "options.chat.opacity",
            Opt::TextBackgroundOpacity => "options.accessibility.text_background_opacity",
            Opt::ChatScale => "options.chat.scale",
            Opt::ChatWidth => "options.chat.width",
            Opt::ChatHeightFocused => "options.chat.height.focused",
            Opt::ChatHeightUnfocused => "options.chat.height.unfocused",
            Opt::ChatLineSpacing => "options.chat.line_spacing",
            Opt::AutoSuggestions => "options.autoSuggestCommands",
            Opt::ReducedDebugInfo => "options.reducedDebugInfo",
            Opt::HighContrast => "options.accessibility.high_contrast",
            Opt::ForceUnicode => "options.forceUnicodeFont",
            Opt::JapaneseGlyphs => "options.japaneseGlyphVariants",
        }
    }
    fn kind(self, s: &Settings) -> Kind {
        let unit = |v: f32, lo: f32, hi: f32| Kind::Slider(((v - lo) / (hi - lo)).clamp(0.0, 1.0));
        match self {
            Opt::Fov => unit(s.fov, 30.0, 110.0),
            Opt::RenderDistance => unit(s.render_distance as f32, 2.0, 32.0),
            Opt::SimulationDistance => unit(s.simulation_distance as f32, 5.0, 32.0),
            Opt::Sensitivity => Kind::Slider(s.sensitivity),
            Opt::WheelSensitivity => unit(s.mouse_wheel_sensitivity, 0.01, 10.0),
            Opt::Volume(i) => Kind::Slider(if i == 0 {
                s.master_volume
            } else {
                s.volumes[i]
            }),
            Opt::GuiScale => unit(s.gui_scale as f32, 0.0, 6.0),
            Opt::Gamma => Kind::Slider(s.gamma),
            Opt::MaxFps => unit(s.max_fps as f32, 10.0, 260.0),
            Opt::CloudRange => unit(s.cloud_range as f32, 2.0, 128.0),
            Opt::EntityDistance => unit(s.entity_distance, 0.5, 5.0),
            Opt::BiomeBlend => unit(s.biome_blend as f32, 0.0, 7.0),
            Opt::Preset => unit(preset_index(&s.graphics_preset) as f32, 0.0, 3.0),
            Opt::Mipmaps => unit(s.mipmap_levels as f32, 0.0, 4.0),
            Opt::WeatherRadius => unit(s.weather_radius as f32, 3.0, 10.0),
            Opt::ChunkFade => unit(s.chunk_fade, 0.0, 2.0),
            Opt::MenuBlur => unit(s.menu_blur as f32, 0.0, 10.0),
            Opt::ChatOpacity => Kind::Slider(s.chat_opacity),
            Opt::TextBackgroundOpacity => Kind::Slider(s.text_background_opacity),
            Opt::ChatScale => Kind::Slider(s.chat_scale),
            Opt::ChatWidth => Kind::Slider(s.chat_width),
            Opt::ChatHeightFocused => Kind::Slider(s.chat_height_focused),
            Opt::ChatHeightUnfocused => Kind::Slider(s.chat_height_unfocused),
            Opt::ChatLineSpacing => Kind::Slider(s.chat_line_spacing),
            _ => Kind::Cycle,
        }
    }
    /// The widget message (OptionInstance.toString / CycleButton full name).
    pub fn label(self, s: &Settings) -> Text {
        let c = self.caption();
        match self {
            Opt::Fov => match s.fov as i32 {
                70 => generic(c, tr("options.fov.min")),
                110 => generic(c, tr("options.fov.max")),
                v => generic(c, Text::literal(v.to_string())),
            },
            Opt::RenderDistance => generic(c, Text::tr("options.chunks", &[&s.render_distance])),
            Opt::SimulationDistance => {
                generic(c, Text::tr("options.chunks", &[&s.simulation_distance]))
            }
            Opt::Sensitivity => {
                if s.sensitivity == 0.0 {
                    generic(c, tr("options.sensitivity.min"))
                } else if s.sensitivity == 1.0 {
                    generic(c, tr("options.sensitivity.max"))
                } else {
                    percent(c, 2.0 * s.sensitivity)
                }
            }
            Opt::WheelSensitivity => generic(
                c,
                Text::literal(format!("{:.2}", s.mouse_wheel_sensitivity)),
            ),
            Opt::Volume(i) => {
                let v = if i == 0 {
                    s.master_volume
                } else {
                    s.volumes[i]
                };
                let caption = format!("soundCategory.{}", SOUND_SOURCES[i]);
                if v == 0.0 {
                    generic(&caption, tr("options.off"))
                } else {
                    percent(&caption, v)
                }
            }
            Opt::GuiScale => {
                if s.gui_scale == 0 {
                    tr("options.guiScale.auto")
                } else {
                    Text::literal(s.gui_scale.to_string())
                }
            }
            Opt::Gamma => match (f64::from(s.gamma) * 100.0) as i32 {
                0 => generic(c, tr("options.gamma.min")),
                50 => generic(c, tr("options.gamma.default")),
                100 => generic(c, tr("options.gamma.max")),
                v => generic(c, Text::literal(v.to_string())),
            },
            Opt::MaxFps => {
                if s.max_fps >= 260 {
                    generic(c, tr("options.framerateLimit.max"))
                } else {
                    generic(c, Text::tr("options.framerate", &[&s.max_fps]))
                }
            }
            Opt::CloudRange => generic(c, Text::tr("options.chunks", &[&s.cloud_range])),
            Opt::EntityDistance => percent(c, s.entity_distance),
            Opt::BiomeBlend => generic(
                c,
                tr(&format!(
                    "options.biomeBlendRadius.{}",
                    s.biome_blend * 2 + 1
                )),
            ),
            Opt::Preset => generic(c, tr(&format!("options.graphics.{}", s.graphics_preset))),
            Opt::Mipmaps => {
                if s.mipmap_levels == 0 {
                    generic(c, tr("options.off"))
                } else {
                    generic(c, Text::literal(s.mipmap_levels.to_string()))
                }
            }
            Opt::WeatherRadius => generic(c, Text::tr("options.blocks", &[&s.weather_radius])),
            Opt::ChunkFade => {
                if s.chunk_fade <= 0.0 {
                    tr("options.chunkFade.none")
                } else {
                    Text::tr(
                        "options.chunkFade.seconds",
                        &[&format!("{:.2}", s.chunk_fade)],
                    )
                }
            }
            Opt::MenuBlur => {
                if s.menu_blur == 0 {
                    generic(c, tr("options.off"))
                } else {
                    generic(c, Text::literal(s.menu_blur.to_string()))
                }
            }
            Opt::ChatOpacity => percent(c, s.chat_opacity * 0.9 + 0.1),
            Opt::TextBackgroundOpacity => percent(c, s.text_background_opacity),
            Opt::ChatScale => {
                if s.chat_scale == 0.0 {
                    generic(c, tr("options.off"))
                } else {
                    percent(c, s.chat_scale)
                }
            }
            Opt::ChatWidth => chat_px(c, (s.chat_width * 280.0 + 40.0).floor()),
            Opt::ChatHeightFocused => chat_px(c, (s.chat_height_focused * 160.0 + 20.0).floor()),
            Opt::ChatHeightUnfocused => {
                chat_px(c, (s.chat_height_unfocused * 160.0 + 20.0).floor())
            }
            Opt::ChatLineSpacing => percent(c, s.chat_line_spacing),
            _ => generic(c, self.value_text(s)),
        }
    }
    fn value_text(self, s: &Settings) -> Text {
        match self {
            Opt::InvertY => on_off(s.invert_y),
            Opt::DiscreteScroll => on_off(s.discrete_scroll),
            Opt::ViewBobbing => on_off(s.view_bobbing),
            Opt::Vsync => on_off(s.vsync),
            Opt::Fullscreen => on_off(s.fullscreen),
            Opt::SmoothLighting => on_off(s.smooth_lighting),
            Opt::Clouds => tr(match s.clouds.as_str() {
                "false" => "options.off",
                "fast" => "options.clouds.fast",
                _ => "options.clouds.fancy",
            }),
            Opt::Particles => tr([
                "options.particles.all",
                "options.particles.decreased",
                "options.particles.minimal",
            ][usize::from(s.particles.min(2))]),
            Opt::EntityShadows => on_off(s.entity_shadows),
            Opt::Vignette => on_off(s.vignette),
            Opt::Autosave => on_off(s.autosave_indicator),
            Opt::CutoutLeaves => on_off(s.cutout_leaves),
            Opt::ImprovedTransparency => on_off(s.improved_transparency),
            Opt::AttackIndicator => tr([
                "options.off",
                "options.attack.crosshair",
                "options.attack.hotbar",
            ][usize::from(s.attack_indicator.min(2))]),
            Opt::InactivityFps => tr(if s.inactivity_fps_limit == "minimized" {
                "options.inactivityFpsLimit.minimized"
            } else {
                "options.inactivityFpsLimit.afk"
            }),
            Opt::ChunkBuilder => tr([
                "options.prioritizeChunkUpdates.none",
                "options.prioritizeChunkUpdates.byPlayer",
                "options.prioritizeChunkUpdates.nearby",
            ][usize::from(s.prioritize_chunk_updates.min(2))]),
            Opt::ToggleCrouch => tr(if s.toggle_crouch {
                "options.key.toggle"
            } else {
                "options.key.hold"
            }),
            Opt::ToggleSprint => tr(if s.toggle_sprint {
                "options.key.toggle"
            } else {
                "options.key.hold"
            }),
            Opt::AutoJump => on_off(s.auto_jump),
            Opt::ChatVisibility => tr([
                "options.chat.visibility.full",
                "options.chat.visibility.system",
                "options.chat.visibility.hidden",
            ][usize::from(s.chat_visibility.min(2))]),
            Opt::ChatColors => on_off(s.chat_colors),
            Opt::ChatLinks => on_off(s.chat_links),
            Opt::AutoSuggestions => on_off(s.auto_suggestions),
            Opt::ReducedDebugInfo => on_off(s.reduced_debug_info),
            Opt::HighContrast => on_off(s.high_contrast),
            Opt::ForceUnicode => on_off(s.force_unicode),
            Opt::JapaneseGlyphs => on_off(s.japanese_glyphs),
            _ => Text::empty(),
        }
    }
    /// Set a slider from its 0..1 position (OptionInstance value sets).
    pub fn set_slider(self, s: &mut Settings, v: f32) {
        let step = |lo: f32, hi: f32| (lo + v * (hi - lo)).round();
        match self {
            Opt::Fov => s.fov = step(30.0, 110.0),
            Opt::RenderDistance => s.render_distance = step(2.0, 32.0) as i32,
            Opt::SimulationDistance => s.simulation_distance = step(5.0, 32.0) as i32,
            Opt::Sensitivity => s.sensitivity = v,
            Opt::WheelSensitivity => s.mouse_wheel_sensitivity = 0.01 + v * 9.99,
            Opt::Volume(0) => s.master_volume = v,
            Opt::Volume(i) => s.volumes[i] = v,
            Opt::GuiScale => s.gui_scale = step(0.0, 6.0) as u32,
            Opt::Gamma => s.gamma = v,
            Opt::MaxFps => s.max_fps = (step(1.0, 26.0) as u32) * 10,
            Opt::CloudRange => s.cloud_range = step(2.0, 128.0) as i32,
            Opt::EntityDistance => s.entity_distance = step(2.0, 20.0) / 4.0,
            Opt::BiomeBlend => s.biome_blend = step(0.0, 7.0) as i32,
            Opt::Preset => s.graphics_preset = PRESETS[step(0.0, 3.0) as usize].into(),
            Opt::Mipmaps => s.mipmap_levels = step(0.0, 4.0) as i32,
            Opt::WeatherRadius => s.weather_radius = step(3.0, 10.0) as i32,
            Opt::ChunkFade => s.chunk_fade = step(0.0, 40.0) / 20.0,
            Opt::MenuBlur => s.menu_blur = step(0.0, 10.0) as i32,
            Opt::ChatOpacity => s.chat_opacity = v,
            Opt::TextBackgroundOpacity => s.text_background_opacity = v,
            Opt::ChatScale => s.chat_scale = v,
            Opt::ChatWidth => s.chat_width = v,
            Opt::ChatHeightFocused => s.chat_height_focused = v,
            Opt::ChatHeightUnfocused => s.chat_height_unfocused = v,
            Opt::ChatLineSpacing => s.chat_line_spacing = v,
            _ => {}
        }
        if self.affects_preset() {
            s.graphics_preset = "custom".into();
        }
    }
    fn affects_preset(self) -> bool {
        matches!(
            self,
            Opt::RenderDistance
                | Opt::SimulationDistance
                | Opt::SmoothLighting
                | Opt::Clouds
                | Opt::CloudRange
                | Opt::Particles
                | Opt::EntityDistance
                | Opt::EntityShadows
                | Opt::BiomeBlend
                | Opt::Mipmaps
                | Opt::WeatherRadius
                | Opt::CutoutLeaves
                | Opt::ImprovedTransparency
                | Opt::MenuBlur
        )
    }
    /// Advance a cycle button (CycleButton.cycleValue).
    pub fn cycle(self, s: &mut Settings, back: bool) {
        let next = |n: u8, count: u8| {
            if back {
                (n + count - 1) % count
            } else {
                (n + 1) % count
            }
        };
        match self {
            Opt::InvertY => s.invert_y = !s.invert_y,
            Opt::DiscreteScroll => s.discrete_scroll = !s.discrete_scroll,
            Opt::ViewBobbing => s.view_bobbing = !s.view_bobbing,
            Opt::Vsync => s.vsync = !s.vsync,
            Opt::Fullscreen => s.fullscreen = !s.fullscreen,
            Opt::SmoothLighting => s.smooth_lighting = !s.smooth_lighting,
            Opt::Clouds => {
                let order = ["false", "fast", "true"];
                let i = order.iter().position(|c| *c == s.clouds).unwrap_or(2) as u8;
                s.clouds = order[usize::from(next(i, 3))].into();
            }
            Opt::Particles => s.particles = next(s.particles, 3),
            Opt::EntityShadows => s.entity_shadows = !s.entity_shadows,
            Opt::Vignette => s.vignette = !s.vignette,
            Opt::Autosave => s.autosave_indicator = !s.autosave_indicator,
            Opt::CutoutLeaves => s.cutout_leaves = !s.cutout_leaves,
            Opt::ImprovedTransparency => s.improved_transparency = !s.improved_transparency,
            Opt::AttackIndicator => s.attack_indicator = next(s.attack_indicator, 3),
            Opt::InactivityFps => {
                s.inactivity_fps_limit = if s.inactivity_fps_limit == "afk" {
                    "minimized".into()
                } else {
                    "afk".into()
                }
            }
            Opt::ChunkBuilder => s.prioritize_chunk_updates = next(s.prioritize_chunk_updates, 3),
            Opt::ToggleCrouch => s.toggle_crouch = !s.toggle_crouch,
            Opt::ToggleSprint => s.toggle_sprint = !s.toggle_sprint,
            Opt::AutoJump => s.auto_jump = !s.auto_jump,
            Opt::ChatVisibility => s.chat_visibility = next(s.chat_visibility, 3),
            Opt::ChatColors => s.chat_colors = !s.chat_colors,
            Opt::ChatLinks => s.chat_links = !s.chat_links,
            Opt::AutoSuggestions => s.auto_suggestions = !s.auto_suggestions,
            Opt::ReducedDebugInfo => s.reduced_debug_info = !s.reduced_debug_info,
            Opt::HighContrast => s.high_contrast = !s.high_contrast,
            Opt::ForceUnicode => s.force_unicode = !s.force_unicode,
            Opt::JapaneseGlyphs => s.japanese_glyphs = !s.japanese_glyphs,
            _ => {}
        }
        if self.affects_preset() {
            s.graphics_preset = "custom".into();
        }
    }
}
const PRESETS: [&str; 4] = ["fast", "fancy", "fabulous", "custom"];
fn preset_index(p: &str) -> usize {
    PRESETS.iter().position(|x| *x == p).unwrap_or(1)
}
/// GraphicsPreset.apply: quality options for Fast/Fancy/Fabulous.
pub fn apply_preset(s: &mut Settings) {
    match s.graphics_preset.as_str() {
        "fast" => {
            s.clouds = "fast".into();
            s.particles = 1;
            s.entity_shadows = false;
            s.biome_blend = 1;
            s.cutout_leaves = false;
            s.improved_transparency = false;
            s.mipmap_levels = 2;
            s.weather_radius = 5;
            s.cloud_range = 64;
            s.entity_distance = 0.75;
        }
        "fancy" | "fabulous" => {
            s.clouds = "true".into();
            s.particles = 0;
            s.entity_shadows = true;
            s.biome_blend = 2;
            s.cutout_leaves = true;
            s.improved_transparency = s.graphics_preset == "fabulous";
            s.mipmap_levels = 4;
            s.weather_radius = 10;
            s.cloud_range = 128;
            s.entity_distance = 1.0;
        }
        _ => {}
    }
}

/// A resource pack entry for the pack screen.
#[derive(Clone, Debug, PartialEq)]
pub struct PackEntry {
    pub id: String,
    pub title: Text,
    pub description: Text,
    pub compatible: bool,
    /// Built-in packs that cannot be removed (vanilla).
    pub required: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Main,
    Video,
    Sound,
    Controls,
    KeyBinds,
    Mouse,
    Chat,
    Accessibility,
    Language,
    Font,
    Packs,
}
#[derive(Clone, Debug, PartialEq)]
enum Act {
    None,
    Push(Page),
    Done,
    Cycle(Opt),
    Key(&'static str),
    ResetKey(&'static str),
    ResetKeys,
    OpenPackFolder,
}
enum WKind {
    Button {
        label: Text,
        active: bool,
        act: Act,
    },
    Slider {
        label: Text,
        value: f32,
        opt: Opt,
    },
    Text {
        text: Text,
        color: u32,
        centered: bool,
    },
    LanguageRow {
        label: Text,
        selected: bool,
        code: String,
    },
    PackRow {
        pack: PackEntry,
        selected: bool,
        index: usize,
        count: usize,
    },
}
struct Widget {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    kind: WKind,
    /// Scrolls with the list (clipped to the list area).
    list: bool,
}
/// What the game must do after an interaction.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// Close the options screens (back to the pause menu or title).
    Close,
    /// Settings changed: apply them.
    Changed,
    /// The language changed (reload translations).
    Language(String),
    /// The pack selection changed (reload resources).
    Packs(Vec<String>),
    OpenPackFolder,
}
pub struct OptionsUi {
    pub stack: Vec<Page>,
    scroll: f32,
    dragging: Option<Opt>,
    /// Key Binds: the mapping waiting for a key.
    pub selected_key: Option<&'static str>,
    pub languages: Vec<LanguageInfo>,
    language_choice: Option<String>,
    language_search: ui::EditBox,
    /// Packs: (available, selected top-first).
    pub available: Vec<PackEntry>,
    pub selected: Vec<PackEntry>,
    packs_changed: bool,
    pub in_world: bool,
    /// Scroll the selected language into the middle on the next layout.
    center_language: bool,
}
impl Default for OptionsUi {
    fn default() -> Self {
        let mut search = ui::EditBox::new(0.0, 0.0, 200.0, 15.0);
        search.max_length = 32;
        Self {
            stack: vec![Page::Main],
            scroll: 0.0,
            dragging: None,
            selected_key: None,
            languages: Vec::new(),
            language_choice: None,
            language_search: search,
            available: Vec::new(),
            selected: Vec::new(),
            packs_changed: false,
            in_world: false,
            center_language: false,
        }
    }
}
fn button(x: f32, y: f32, w: f32, label: Text, act: Act) -> Widget {
    Widget {
        x,
        y,
        w,
        h: 20.0,
        kind: WKind::Button {
            label,
            active: true,
            act,
        },
        list: false,
    }
}
fn inactive(mut w: Widget) -> Widget {
    if let WKind::Button { active, .. } = &mut w.kind {
        *active = false;
    }
    w
}
fn option_widget(s: &Settings, opt: Opt, x: f32, y: f32, w: f32) -> Widget {
    match opt.kind(s) {
        Kind::Slider(v) => Widget {
            x,
            y,
            w,
            h: 20.0,
            kind: WKind::Slider {
                label: opt.label(s),
                value: v,
                opt,
            },
            list: false,
        },
        Kind::Cycle => button(x, y, w, opt.label(s), Act::Cycle(opt)),
    }
}
/// Rows of an OptionsList: (big?, options) or a header.
enum Row {
    Big(Opt),
    Pair(Opt, Option<Opt>),
    Header(&'static str),
    Buttons(Widget, Option<Widget>),
}
fn pairs(opts: &[Opt]) -> Vec<Row> {
    opts.chunks(2)
        .map(|c| Row::Pair(c[0], c.get(1).copied()))
        .collect()
}
impl OptionsUi {
    pub fn page(&self) -> Page {
        *self.stack.last().unwrap_or(&Page::Main)
    }
    /// Jump straight to a page (control socket / tests).
    pub fn show(&mut self, page: Page) {
        if page != Page::Main {
            self.push(page);
        }
    }
    pub fn open(&mut self, in_world: bool) {
        self.stack = vec![Page::Main];
        self.scroll = 0.0;
        self.in_world = in_world;
        self.selected_key = None;
    }
    fn push(&mut self, page: Page) {
        self.stack.push(page);
        self.scroll = 0.0;
        if page == Page::Language {
            self.language_choice = None;
            self.language_search.set_value("");
            self.center_language = true;
        }
        if page == Page::Packs {
            self.packs_changed = false;
        }
    }
    fn title(page: Page) -> &'static str {
        match page {
            Page::Main => "options.title",
            Page::Video => "options.videoTitle",
            Page::Sound => "options.sounds.title",
            Page::Controls => "controls.title",
            Page::KeyBinds => "controls.keybinds.title",
            Page::Mouse => "options.mouse_settings.title",
            Page::Chat => "options.chat.title",
            Page::Accessibility => "options.accessibility.title",
            Page::Language => "options.language.title",
            Page::Font => "options.font.title",
            Page::Packs => "resourcePack.title",
        }
    }
    fn list_rows(&self, s: &Settings) -> Vec<Row> {
        match self.page() {
            Page::Video => {
                let mut rows = vec![Row::Header("options.video.display.header")];
                rows.extend(pairs(&[
                    Opt::MaxFps,
                    Opt::Vsync,
                    Opt::InactivityFps,
                    Opt::GuiScale,
                    Opt::Fullscreen,
                    Opt::Gamma,
                ]));
                rows.push(Row::Header("options.video.quality.header"));
                rows.push(Row::Big(Opt::Preset));
                rows.extend(pairs(&[
                    Opt::BiomeBlend,
                    Opt::RenderDistance,
                    Opt::ChunkBuilder,
                    Opt::SimulationDistance,
                    Opt::SmoothLighting,
                    Opt::Clouds,
                    Opt::Particles,
                    Opt::Mipmaps,
                    Opt::EntityShadows,
                    Opt::EntityDistance,
                    Opt::MenuBlur,
                    Opt::CloudRange,
                    Opt::CutoutLeaves,
                    Opt::ImprovedTransparency,
                    Opt::WeatherRadius,
                ]));
                rows.push(Row::Header("options.video.preferences.header"));
                rows.extend(pairs(&[
                    Opt::Autosave,
                    Opt::Vignette,
                    Opt::AttackIndicator,
                    Opt::ChunkFade,
                ]));
                rows
            }
            Page::Sound => {
                let mut rows = vec![Row::Big(Opt::Volume(0))];
                let others: Vec<Opt> = (1..SOUND_SOURCES.len()).map(Opt::Volume).collect();
                rows.extend(pairs(&others));
                rows
            }
            Page::Controls => {
                let mut rows = vec![Row::Buttons(
                    button(
                        0.0,
                        0.0,
                        150.0,
                        tr("options.mouse_settings"),
                        Act::Push(Page::Mouse),
                    ),
                    Some(button(
                        0.0,
                        0.0,
                        150.0,
                        tr("controls.keybinds"),
                        Act::Push(Page::KeyBinds),
                    )),
                )];
                rows.extend(pairs(&[
                    Opt::ToggleCrouch,
                    Opt::ToggleSprint,
                    Opt::AutoJump,
                ]));
                rows
            }
            Page::Mouse => pairs(&[
                Opt::Sensitivity,
                Opt::WheelSensitivity,
                Opt::DiscreteScroll,
                Opt::InvertY,
            ]),
            Page::Chat => pairs(&[
                Opt::ChatVisibility,
                Opt::ChatColors,
                Opt::ChatLinks,
                Opt::ChatOpacity,
                Opt::TextBackgroundOpacity,
                Opt::ChatScale,
                Opt::ChatLineSpacing,
                Opt::ChatWidth,
                Opt::ChatHeightFocused,
                Opt::ChatHeightUnfocused,
                Opt::AutoSuggestions,
                Opt::ReducedDebugInfo,
            ]),
            Page::Accessibility => pairs(&[
                Opt::HighContrast,
                Opt::MenuBlur,
                Opt::TextBackgroundOpacity,
                Opt::ChatOpacity,
                Opt::ChatLineSpacing,
                Opt::ViewBobbing,
                Opt::ToggleCrouch,
                Opt::ToggleSprint,
                Opt::AutoJump,
            ]),
            Page::Font => pairs(&[Opt::ForceUnicode, Opt::JapaneseGlyphs]),
            _ => {
                let _ = s;
                Vec::new()
            }
        }
    }
    /// Build every widget of the current page.
    fn layout(
        &mut self,
        s: &Settings,
        atlas: &HudAtlas,
        w: f32,
        h: f32,
    ) -> (Vec<Widget>, f32, f32, f32) {
        let mut out = Vec::new();
        let page = self.page();
        let cx = (w / 2.0).floor();
        let done = |y: f32| button(cx - 100.0, y, 200.0, tr("gui.done"), Act::Done);
        // (list top, list bottom, content height)
        let (mut top, mut bottom, mut content) = (33.0, h - 33.0, 0.0);
        match page {
            Page::Main => {
                out.push(Widget {
                    x: 0.0,
                    y: 12.0,
                    w,
                    h: 9.0,
                    kind: WKind::Text {
                        text: tr("options.title"),
                        color: u32::MAX,
                        centered: true,
                    },
                    list: false,
                });
                out.push(option_widget(s, Opt::Fov, cx - 154.0, 29.0, 150.0));
                out.push(inactive(button(
                    cx + 4.0,
                    29.0,
                    150.0,
                    tr("options.online"),
                    Act::None,
                )));
                let entries = [
                    ("options.skinCustomisation", None),
                    ("options.sounds", Some(Page::Sound)),
                    ("options.video", Some(Page::Video)),
                    ("options.controls", Some(Page::Controls)),
                    ("options.language", Some(Page::Language)),
                    ("options.chat", Some(Page::Chat)),
                    ("options.resourcepack", Some(Page::Packs)),
                    ("options.accessibility", Some(Page::Accessibility)),
                    ("options.telemetry", None),
                    ("options.credits_and_attribution", None),
                ];
                let gy = (61.0 + 30.0f32).min(h - 33.0 - 120.0);
                for (i, (key, page)) in entries.iter().enumerate() {
                    let x = cx - 158.0 + 4.0 + (i % 2) as f32 * 158.0;
                    let y = gy + (i / 2) as f32 * 24.0;
                    let b = button(x, y, 150.0, tr(key), page.map_or(Act::None, Act::Push));
                    out.push(if page.is_some() { b } else { inactive(b) });
                }
                out.push(done(h - 27.0));
                top = 0.0;
                bottom = 0.0;
            }
            Page::KeyBinds => {
                out.push(Widget {
                    x: 0.0,
                    y: 12.0,
                    w,
                    h: 9.0,
                    kind: WKind::Text {
                        text: tr(Self::title(page)),
                        color: u32::MAX,
                        centered: true,
                    },
                    list: false,
                });
                let mut mappings: Vec<(&str, &str)> =
                    MAPPINGS.iter().map(|(n, _, c)| (*n, *c)).collect();
                mappings.sort_by(|a, b| {
                    let ca = CATEGORIES.iter().position(|c| *c == a.1).unwrap_or(99);
                    let cb = CATEGORIES.iter().position(|c| *c == b.1).unwrap_or(99);
                    ca.cmp(&cb)
                        .then_with(|| lang::get(a.0).cmp(&lang::get(b.0)))
                });
                let row_left = cx - 170.0;
                let scrollbar_x = row_left + 340.0 + 6.0 + 2.0;
                let list_start = out.len();
                let mut y = top + 2.0 - self.scroll;
                let mut last_category = "";
                for (name, category) in mappings {
                    if category != last_category {
                        last_category = category;
                        let label = Text::translate(&format!("key.categories.{category}"), &[])
                            .with_style(&Style {
                                bold: Some(true),
                                underlined: Some(true),
                                ..Style::default()
                            });
                        out.push(Widget {
                            x: 0.0,
                            y: y + 7.0,
                            w,
                            h: 9.0,
                            kind: WKind::Text {
                                text: label,
                                color: u32::MAX,
                                centered: true,
                            },
                            list: false,
                        });
                        y += 20.0;
                    }
                    out.push(Widget {
                        x: row_left + 2.0,
                        y: y + 6.0,
                        w: 200.0,
                        h: 9.0,
                        kind: WKind::Text {
                            text: tr(name),
                            color: u32::MAX,
                            centered: false,
                        },
                        list: false,
                    });
                    let key = s.keys.get(name).to_owned();
                    let mut message = keybinds::display_name(&key);
                    let conflicts = s.keys.conflicts(name);
                    if !conflicts.is_empty() {
                        message = Text::literal("[ ")
                            .append(message.with_color(text::WHITE))
                            .append(" ]")
                            .with_color(text::YELLOW);
                    }
                    if self.selected_key == Some(name) {
                        message = Text::literal("> ")
                            .append(message.with_style(&Style {
                                color: Some(text::WHITE),
                                underlined: Some(true),
                                ..Style::default()
                            }))
                            .append(" <")
                            .with_color(text::YELLOW);
                    }
                    let reset_x = scrollbar_x - 50.0 - 10.0;
                    let change_x = reset_x - 5.0 - 75.0;
                    let name_static = MAPPINGS.iter().find(|m| m.0 == name).map_or("", |m| m.0);
                    out.push(button(change_x, y, 75.0, message, Act::Key(name_static)));
                    if !conflicts.is_empty() {
                        out.push(Widget {
                            x: change_x - 6.0,
                            y: y + 1.0,
                            w: 3.0,
                            h: 18.0,
                            kind: WKind::Text {
                                text: Text::empty(),
                                color: rgba(255, 255, 0, 255),
                                centered: false,
                            },
                            list: false,
                        });
                    }
                    let reset = button(
                        reset_x,
                        y,
                        50.0,
                        tr("controls.reset"),
                        Act::ResetKey(name_static),
                    );
                    out.push(if s.keys.is_default(name) {
                        inactive(reset)
                    } else {
                        reset
                    });
                    y += 20.0;
                }
                content = y + self.scroll - top + 2.0;
                out[list_start..].iter_mut().for_each(|w| w.list = true);
                out.push(button(
                    cx - 154.0,
                    h - 27.0,
                    150.0,
                    tr("controls.resetAll"),
                    Act::ResetKeys,
                ));
                out.push(button(cx + 4.0, h - 27.0, 150.0, tr("gui.done"), Act::Done));
            }
            Page::Language => {
                top = 36.0;
                bottom = h - 53.0;
                out.push(Widget {
                    x: 0.0,
                    y: 4.0,
                    w,
                    h: 9.0,
                    kind: WKind::Text {
                        text: tr(Self::title(page)),
                        color: u32::MAX,
                        centered: true,
                    },
                    list: false,
                });
                self.language_search.x = cx - 100.0;
                self.language_search.y = 17.0;
                let query = self.language_search.value.to_lowercase();
                let current = self
                    .language_choice
                    .clone()
                    .unwrap_or_else(|| s.language.clone());
                if std::mem::take(&mut self.center_language)
                    && let Some(i) = self.languages.iter().position(|l| l.code == current)
                {
                    // AbstractSelectionList.centerScrollOn.
                    self.scroll = (i as f32 * 18.0 + 9.0 - (bottom - top) / 2.0).max(0.0);
                }
                let list_start = out.len();
                let mut y = top + 2.0 - self.scroll;
                for l in &self.languages {
                    if !query.is_empty()
                        && !l.label().to_lowercase().contains(&query)
                        && !l.code.contains(&query)
                    {
                        continue;
                    }
                    out.push(Widget {
                        x: cx - 110.0,
                        y,
                        w: 220.0,
                        h: 18.0,
                        kind: WKind::LanguageRow {
                            label: Text::literal(l.label()),
                            selected: l.code == current,
                            code: l.code.clone(),
                        },
                        list: false,
                    });
                    y += 18.0;
                }
                content = y + self.scroll - top + 2.0;
                out[list_start..].iter_mut().for_each(|w| w.list = true);
                out.push(Widget {
                    x: 0.0,
                    y: h - 45.0,
                    w,
                    h: 9.0,
                    kind: WKind::Text {
                        text: tr("options.languageAccuracyWarning"),
                        color: rgba(0xBA, 0xBA, 0xBA, 255),
                        centered: true,
                    },
                    list: false,
                });
                out.push(button(
                    cx - 154.0,
                    h - 28.0,
                    150.0,
                    tr("options.font"),
                    Act::Push(Page::Font),
                ));
                out.push(button(cx + 4.0, h - 28.0, 150.0, tr("gui.done"), Act::Done));
            }
            Page::Packs => {
                top = 47.0;
                bottom = h - 33.0;
                out.push(Widget {
                    x: 0.0,
                    y: 8.0,
                    w,
                    h: 9.0,
                    kind: WKind::Text {
                        text: tr("resourcePack.title"),
                        color: u32::MAX,
                        centered: true,
                    },
                    list: false,
                });
                out.push(Widget {
                    x: 0.0,
                    y: 20.0,
                    w,
                    h: 9.0,
                    kind: WKind::Text {
                        text: tr("pack.dropInfo").with_color(text::GRAY),
                        color: u32::MAX,
                        centered: true,
                    },
                    list: false,
                });
                let lists = [(false, cx - 4.0 - 200.0), (true, cx + 4.0)];
                let list_start = out.len();
                for (selected, lx) in lists {
                    let entries = if selected {
                        &self.selected
                    } else {
                        &self.available
                    };
                    let header = tr(if selected {
                        "pack.selected.title"
                    } else {
                        "pack.available.title"
                    })
                    .with_style(&Style {
                        bold: Some(true),
                        underlined: Some(true),
                        ..Style::default()
                    });
                    out.push(Widget {
                        x: lx,
                        y: top + 4.0,
                        w: 200.0,
                        h: 9.0,
                        kind: WKind::Text {
                            text: header,
                            color: u32::MAX,
                            centered: true,
                        },
                        list: false,
                    });
                    let mut y = top + 17.0 - self.scroll;
                    for (i, p) in entries.iter().enumerate() {
                        out.push(Widget {
                            x: lx,
                            y,
                            w: 200.0,
                            h: 36.0,
                            kind: WKind::PackRow {
                                pack: p.clone(),
                                selected,
                                index: i,
                                count: entries.len(),
                            },
                            list: false,
                        });
                        y += 36.0;
                    }
                    content = f32::max(content, y + self.scroll - top + 4.0);
                }
                out[list_start..].iter_mut().for_each(|w| w.list = true);
                out.push(button(
                    cx - 154.0,
                    h - 27.0,
                    150.0,
                    tr("pack.openFolder"),
                    Act::OpenPackFolder,
                ));
                out.push(button(cx + 4.0, h - 27.0, 150.0, tr("gui.done"), Act::Done));
            }
            _ => {
                out.push(Widget {
                    x: 0.0,
                    y: 12.0,
                    w,
                    h: 9.0,
                    kind: WKind::Text {
                        text: tr(Self::title(page)),
                        color: u32::MAX,
                        centered: true,
                    },
                    list: false,
                });
                let list_start = out.len();
                let mut y = top + 2.0 - self.scroll;
                for row in self.list_rows(s) {
                    match row {
                        Row::Header(key) => {
                            let padding = if y + self.scroll <= top + 2.0 {
                                0.0
                            } else {
                                18.0
                            };
                            let label = tr(key).with_style(&Style {
                                bold: Some(true),
                                underlined: Some(true),
                                ..Style::default()
                            });
                            out.push(Widget {
                                x: 0.0,
                                y: y + 2.0 + padding + 4.0,
                                w,
                                h: 9.0,
                                kind: WKind::Text {
                                    text: label,
                                    color: u32::MAX,
                                    centered: true,
                                },
                                list: false,
                            });
                            y += padding + 18.0;
                        }
                        Row::Big(opt) => {
                            out.push(option_widget(s, opt, cx - 155.0, y + 2.0, 310.0));
                            y += 25.0;
                        }
                        Row::Pair(a, b) => {
                            out.push(option_widget(s, a, cx - 155.0, y + 2.0, 150.0));
                            if let Some(b) = b {
                                out.push(option_widget(s, b, cx + 5.0, y + 2.0, 150.0));
                            }
                            y += 25.0;
                        }
                        Row::Buttons(mut a, b) => {
                            a.x = cx - 155.0;
                            a.y = y + 2.0;
                            out.push(a);
                            if let Some(mut b) = b {
                                b.x = cx + 5.0;
                                b.y = y + 2.0;
                                out.push(b);
                            }
                            y += 25.0;
                        }
                    }
                }
                content = y + self.scroll - top + 2.0;
                out[list_start..].iter_mut().for_each(|w| w.list = true);
                out.push(done(h - 27.0));
            }
        }
        let _ = atlas;
        (out, top, bottom, content)
    }
    fn clamp_scroll(&mut self, top: f32, bottom: f32, content: f32) {
        let max = (content - (bottom - top)).max(0.0);
        self.scroll = self.scroll.clamp(0.0, max);
    }
    pub fn draw(
        &mut self,
        s: &Settings,
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        cursor: (f32, f32),
    ) {
        let (w, h) = (batch.width, batch.height);
        let (widgets, top, bottom, content) = self.layout(s, atlas, w, h);
        self.clamp_scroll(top, bottom, content);
        let in_world = self.in_world;
        ui::menu_background(batch, atlas, 0.0, 0.0, w, h, in_world);
        let list = bottom > top;
        if list {
            ui::list_background(batch, atlas, top, bottom, in_world);
        }
        let visible = |wd: &Widget| !wd.list || (wd.y >= top && wd.y + wd.h <= bottom);
        let mut tooltip = None;
        for wd in &widgets {
            if !visible(wd) {
                continue;
            }
            let hover = ui::hovered(cursor, wd.x, wd.y, wd.w, wd.h)
                && (!wd.list || cursor.1 >= top && cursor.1 < bottom);
            match &wd.kind {
                WKind::Button { label, active, act } => {
                    ui::button(batch, atlas, wd.x, wd.y, wd.w, wd.h, label, *active, hover);
                    if hover && let Act::Key(name) = act {
                        let conflicts = s.keys.conflicts(name);
                        if !conflicts.is_empty() {
                            let names = conflicts
                                .iter()
                                .map(|c| lang::get(c))
                                .collect::<Vec<_>>()
                                .join(", ");
                            let full = Text::translate(
                                "controls.keybinds.duplicateKeybinds",
                                &[Text::literal(names)],
                            );
                            tooltip = Some(
                                full.plain()
                                    .split('\n')
                                    .map(|l| Text::literal(l.to_owned()))
                                    .collect::<Vec<_>>(),
                            );
                        }
                    }
                }
                WKind::Slider { label, value, opt } => {
                    let dragging = self.dragging == Some(*opt);
                    ui::slider(
                        batch,
                        atlas,
                        wd.x,
                        wd.y,
                        wd.w,
                        label,
                        *value,
                        hover || dragging,
                        true,
                    );
                }
                WKind::Text {
                    text,
                    color,
                    centered,
                } => {
                    if text.is_empty() {
                        batch.fill(atlas, wd.x, wd.y, wd.w, wd.h, *color);
                    } else if *centered {
                        let tw = atlas.font.width_text(text);
                        batch.text_styled(
                            atlas,
                            text,
                            wd.x + ((wd.w - tw) / 2.0).floor(),
                            wd.y,
                            *color,
                            true,
                        );
                    } else {
                        batch.text_styled(atlas, text, wd.x, wd.y, *color, true);
                    }
                }
                WKind::LanguageRow {
                    label, selected, ..
                } => {
                    if *selected {
                        batch.fill(atlas, wd.x, wd.y, wd.w, wd.h, u32::MAX);
                        batch.fill(
                            atlas,
                            wd.x + 1.0,
                            wd.y + 1.0,
                            wd.w - 2.0,
                            wd.h - 2.0,
                            rgba(0, 0, 0, 255),
                        );
                    }
                    let tw = atlas.font.width_text(label);
                    batch.text_styled(
                        atlas,
                        label,
                        wd.x + ((wd.w - tw) / 2.0).floor(),
                        wd.y + 2.0 + 7.0 - 4.0,
                        u32::MAX,
                        true,
                    );
                }
                WKind::PackRow {
                    pack,
                    selected,
                    index,
                    count,
                } => {
                    if hover {
                        batch.fill(atlas, wd.x, wd.y, wd.w, wd.h, rgba(255, 255, 255, 255));
                        batch.fill(
                            atlas,
                            wd.x + 1.0,
                            wd.y + 1.0,
                            wd.w - 2.0,
                            wd.h - 2.0,
                            rgba(0, 0, 0, 255),
                        );
                    }
                    batch.sprite_scaled(
                        atlas,
                        "misc/unknown_pack",
                        wd.x + 2.0,
                        wd.y + 2.0,
                        32.0,
                        32.0,
                        u32::MAX,
                    );
                    let title_color = if pack.compatible {
                        u32::MAX
                    } else {
                        rgba(255, 85, 85, 255)
                    };
                    batch.text_styled(
                        atlas,
                        &pack.title,
                        wd.x + 36.0,
                        wd.y + 3.0,
                        title_color,
                        true,
                    );
                    for (i, line) in atlas
                        .font
                        .split(&pack.description, 157.0)
                        .iter()
                        .take(2)
                        .enumerate()
                    {
                        batch.text_styled(
                            atlas,
                            line,
                            wd.x + 36.0,
                            wd.y + 14.0 + i as f32 * 9.0,
                            rgba(128, 128, 128, 255),
                            true,
                        );
                    }
                    if hover && cursor.0 < wd.x + 34.0 {
                        // TransferableSelectionList arrows over the icon.
                        batch.fill(
                            atlas,
                            wd.x + 2.0,
                            wd.y + 2.0,
                            32.0,
                            32.0,
                            rgba(160, 160, 160, 136),
                        );
                        let sprite_dir = if *selected { "unselect" } else { "select" };
                        let left = cursor.0 < wd.x + 18.0;
                        let name = format!(
                            "gui/sprites/transferable_list/{sprite_dir}{}",
                            if left || !*selected {
                                "_highlighted"
                            } else {
                                ""
                            }
                        );
                        batch.sprite(atlas, &name, wd.x + 2.0, wd.y + 2.0);
                        if *selected {
                            if *index > 0 {
                                batch.sprite(
                                    atlas,
                                    "gui/sprites/transferable_list/move_up",
                                    wd.x + 2.0,
                                    wd.y + 2.0,
                                );
                            }
                            if index + 1 < *count {
                                batch.sprite(
                                    atlas,
                                    "gui/sprites/transferable_list/move_down",
                                    wd.x + 2.0,
                                    wd.y + 2.0,
                                );
                            }
                        }
                    }
                }
            }
        }
        if list {
            ui::separators(batch, atlas, top, bottom, in_world);
            let sx = match self.page() {
                Page::KeyBinds => (w / 2.0).floor() + 170.0 + 8.0,
                Page::Language => (w / 2.0).floor() + 110.0 + 8.0,
                _ => (w / 2.0).floor() + 155.0 + 8.0,
            };
            ui::scrollbar(batch, atlas, sx, top, bottom, content, self.scroll);
        }
        if self.page() == Page::Language {
            let mut search = self.language_search.clone();
            search.draw(batch, atlas, None);
            if search.value.is_empty() {
                batch.text_styled(
                    atlas,
                    &tr("gui.language.search").with_color(text::GRAY).italic(),
                    search.text_x(),
                    search.text_y(),
                    u32::MAX,
                    true,
                );
            }
            self.language_search = search;
        }
        if let Some(lines) = tooltip {
            ui::tooltip(batch, atlas, &lines, cursor);
        }
    }
    /// Mouse press/drag. Returns events for the game.
    pub fn click(
        &mut self,
        s: &mut Settings,
        atlas: &HudAtlas,
        x: f32,
        y: f32,
        size: (f32, f32),
        drag: bool,
        right: bool,
    ) -> Vec<Event> {
        let (widgets, top, bottom, _) = self.layout(s, atlas, size.0, size.1);
        let mut events = Vec::new();
        if drag {
            if let Some(opt) = self.dragging
                && let Some(wd) = widgets
                    .iter()
                    .find(|wd| matches!(&wd.kind, WKind::Slider { opt: o, .. } if *o == opt))
            {
                opt.set_slider(s, ui::slider_value(wd.x, wd.w, x));
                events.push(Event::Changed);
            }
            return events;
        }
        // Key Binds: a mouse button pressed while waiting binds it.
        if let Some(name) = self.selected_key.take() {
            let button = if right {
                "key.mouse.right"
            } else {
                "key.mouse.left"
            };
            s.keys.set(name, button);
            events.push(Event::Changed);
            return events;
        }
        for wd in widgets {
            if !ui::hovered((x, y), wd.x, wd.y, wd.w, wd.h) {
                continue;
            }
            if wd.list && (y < top || y >= bottom || wd.y < top || wd.y + wd.h > bottom) {
                continue;
            }
            match wd.kind {
                WKind::Button {
                    active: true, act, ..
                } => match act {
                    Act::Push(p) => self.push(p),
                    Act::Done => events.extend(self.back(s)),
                    Act::Cycle(opt) => {
                        opt.cycle(s, right);
                        if opt == Opt::Preset {
                            apply_preset(s);
                        }
                        events.push(Event::Changed);
                    }
                    Act::Key(name) => self.selected_key = Some(name),
                    Act::ResetKey(name) => {
                        s.keys.set(name, KeyBinds::default_of(name));
                        events.push(Event::Changed);
                    }
                    Act::ResetKeys => {
                        s.keys.reset_all();
                        events.push(Event::Changed);
                    }
                    Act::OpenPackFolder => events.push(Event::OpenPackFolder),
                    Act::None => {}
                },
                WKind::Slider { opt, .. } => {
                    self.dragging = Some(opt);
                    opt.set_slider(s, ui::slider_value(wd.x, wd.w, x));
                    if opt == Opt::Preset {
                        apply_preset(s);
                    }
                    events.push(Event::Changed);
                }
                WKind::LanguageRow { code, .. } => {
                    if self.language_choice.as_deref() == Some(code.as_str()) {
                        events.extend(self.back(s));
                    } else {
                        self.language_choice = Some(code);
                    }
                }
                WKind::PackRow {
                    pack,
                    selected,
                    index,
                    ..
                } => {
                    if pack.required {
                        continue;
                    }
                    let on_icon = x < wd.x + 34.0;
                    if selected && on_icon && x >= wd.x + 18.0 {
                        // Right half of the icon: move up (top) or down (bottom).
                        let up = y < wd.y + 18.0;
                        let target = if up {
                            index.checked_sub(1)
                        } else {
                            Some(index + 1)
                        };
                        if let Some(t) = target
                            .filter(|t| *t < self.selected.len() && !self.selected[*t].required)
                        {
                            self.selected.swap(index, t);
                            self.packs_changed = true;
                        }
                    } else if selected {
                        let p = self.selected.remove(index);
                        self.available.insert(0, p);
                        self.packs_changed = true;
                    } else {
                        let p = self.available.remove(index);
                        self.selected.insert(0, p);
                        self.packs_changed = true;
                    }
                }
                _ => {}
            }
            break;
        }
        events
    }
    pub fn release(&mut self) {
        self.dragging = None;
    }
    pub fn scroll(&mut self, amount: f32) {
        self.scroll -= amount * 18.0;
    }
    /// Escape or Done: leave the current page, applying page results.
    pub fn back(&mut self, s: &mut Settings) -> Vec<Event> {
        let mut events = Vec::new();
        let page = self.stack.pop().unwrap_or(Page::Main);
        self.scroll = 0.0;
        self.dragging = None;
        self.selected_key = None;
        match page {
            Page::Language => {
                if let Some(code) = self.language_choice.take()
                    && code != s.language
                {
                    s.language = code.clone();
                    events.push(Event::Language(code));
                }
            }
            Page::Packs if self.packs_changed => {
                let mut ids: Vec<String> =
                    self.selected.iter().rev().map(|p| p.id.clone()).collect();
                if !ids.contains(&"vanilla".to_owned()) {
                    ids.insert(0, "vanilla".into());
                }
                s.resource_packs = ids.clone();
                events.push(Event::Packs(ids));
            }
            _ => {}
        }
        events.push(Event::Changed);
        if self.stack.is_empty() {
            events.push(Event::Close);
        }
        events
    }
    /// Typed text (the language search box).
    pub fn type_text(&mut self, text: &str) {
        if self.page() == Page::Language {
            self.language_search.insert_text(text);
            self.scroll = 0.0;
        }
    }
    /// Keys while the options are open. Returns events; `handled` false lets
    /// Escape fall through.
    pub fn key(
        &mut self,
        s: &mut Settings,
        key: winit::keyboard::KeyCode,
        text: Option<&str>,
    ) -> Vec<Event> {
        use winit::keyboard::KeyCode as K;
        if let Some(name) = self.selected_key.take() {
            // KeyBindsScreen.keyPressed: Escape unbinds.
            let value = if key == K::Escape {
                "key.keyboard.unknown".to_owned()
            } else {
                keybinds::key_name(key)
                    .unwrap_or("key.keyboard.unknown")
                    .to_owned()
            };
            s.keys.set(name, &value);
            return vec![Event::Changed];
        }
        if key == K::Escape {
            return self.back(s);
        }
        if self.page() == Page::Language {
            if !self.language_search.key(key, false, false)
                && let Some(t) = text
            {
                let typed: String = t.chars().filter(|c| !c.is_control()).collect();
                if !typed.is_empty() {
                    self.language_search.insert_text(&typed);
                }
            }
            self.scroll = 0.0;
        }
        Vec::new()
    }
}
