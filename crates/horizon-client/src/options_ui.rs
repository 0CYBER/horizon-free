//! The options screens: OptionsScreen, Video Settings, Music & Sounds,
//! Controls, Key Binds, Mouse Settings, Chat Settings, Accessibility,
//! Language and Resource Packs, laid out like vanilla's HeaderAndFooter /
//! OptionsList screens and drawn with the shared widgets.
use crate::{
    gamepad::{self, GlyphStyle},
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
    ShowSubtitles,
    NotificationDisplayTime,
    ForceUnicode,
    JapaneseGlyphs,
    /// Horizon's controller settings (`horizon-controller.txt`).
    PadEnabled,
    PadVibration,
    PadLookSensitivity,
    PadInvertX,
    PadInvertY,
    PadToggleSneak,
    PadMoveDeadzone,
    PadLookDeadzone,
    PadGlyphs,
    PadMenuNav,
    PadKeyboard,
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
pub(crate) enum Kind {
    /// Slider position 0..1.
    Slider(f32),
    Cycle,
}
impl Opt {
    pub(crate) fn caption(self) -> &'static str {
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
            Opt::ShowSubtitles => "options.showSubtitles",
            Opt::NotificationDisplayTime => "options.notifications.display_time",
            Opt::ForceUnicode => "options.forceUnicodeFont",
            Opt::JapaneseGlyphs => "options.japaneseGlyphVariants",
            // Horizon's own options: English captions (no vanilla keys).
            Opt::PadEnabled => "Controller",
            Opt::PadVibration => "Vibration",
            Opt::PadLookSensitivity => "Look Sensitivity",
            Opt::PadInvertX => "Invert Look X",
            Opt::PadInvertY => "Invert Look Y",
            Opt::PadToggleSneak => "Sneak Button",
            Opt::PadMoveDeadzone => "Move Deadzone",
            Opt::PadLookDeadzone => "Look Deadzone",
            Opt::PadGlyphs => "Button Glyphs",
            Opt::PadMenuNav => "Menu Navigation",
            Opt::PadKeyboard => "On-Screen Keyboard",
        }
    }
    pub(crate) fn kind(self, s: &Settings) -> Kind {
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
            Opt::NotificationDisplayTime => unit(s.notification_display_time, 0.5, 10.0),
            Opt::ChatOpacity => Kind::Slider(s.chat_opacity),
            Opt::TextBackgroundOpacity => Kind::Slider(s.text_background_opacity),
            Opt::ChatScale => Kind::Slider(s.chat_scale),
            Opt::ChatWidth => Kind::Slider(s.chat_width),
            Opt::ChatHeightFocused => Kind::Slider(s.chat_height_focused),
            Opt::ChatHeightUnfocused => Kind::Slider(s.chat_height_unfocused),
            Opt::ChatLineSpacing => Kind::Slider(s.chat_line_spacing),
            Opt::PadLookSensitivity => Kind::Slider(s.controller.look_sensitivity),
            Opt::PadMoveDeadzone => unit(s.controller.move_deadzone, 0.0, 0.5),
            Opt::PadLookDeadzone => unit(s.controller.look_deadzone, 0.0, 0.5),
            _ => Kind::Cycle,
        }
    }
    /// A slider's position (0..1), or `None` for a cycle button.
    pub fn slider_position(self, s: &Settings) -> Option<f32> {
        match self.kind(s) {
            Kind::Slider(v) => Some(v),
            Kind::Cycle => None,
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
                let value = if s.gui_scale == 0 {
                    tr("options.guiScale.auto")
                } else {
                    Text::literal(s.gui_scale.to_string())
                };
                generic(c, value)
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
            Opt::NotificationDisplayTime => generic(
                c,
                Text::tr(
                    "options.multiplier",
                    &[&format!("{:.1}", s.notification_display_time)],
                ),
            ),
            Opt::PadLookSensitivity => percent(c, 2.0 * s.controller.look_sensitivity),
            Opt::PadMoveDeadzone | Opt::PadLookDeadzone => {
                let v = if self == Opt::PadMoveDeadzone {
                    s.controller.move_deadzone
                } else {
                    s.controller.look_deadzone
                };
                Text::translate(
                    "options.percent_value",
                    &[
                        tr(c),
                        Text::literal(((v * 100.0).round() as i32).to_string()),
                    ],
                )
            }
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
            Opt::ShowSubtitles => on_off(s.show_subtitles),
            Opt::ForceUnicode => on_off(s.force_unicode),
            Opt::JapaneseGlyphs => on_off(s.japanese_glyphs),
            Opt::PadEnabled => on_off(s.controller.enabled),
            Opt::PadVibration => on_off(s.controller.vibration),
            Opt::PadInvertX => on_off(s.controller.invert_x),
            Opt::PadInvertY => on_off(s.controller.invert_y),
            Opt::PadToggleSneak => tr(if s.controller.toggle_sneak {
                "options.key.toggle"
            } else {
                "options.key.hold"
            }),
            Opt::PadGlyphs => Text::literal(match s.controller.glyphs {
                GlyphStyle::Auto => "Auto",
                GlyphStyle::Xbox => "Xbox",
                GlyphStyle::PlayStation => "PlayStation",
                GlyphStyle::Nintendo => "Nintendo",
            }),
            Opt::PadMenuNav => Text::literal(match s.controller.menu_nav {
                gamepad::MenuNav::Focus => "Focus",
                gamepad::MenuNav::Pointer => "Pointer",
            }),
            Opt::PadKeyboard => Text::literal(match s.controller.keyboard {
                gamepad::TextKeyboard::Auto => "Auto",
                gamepad::TextKeyboard::Horizon => "Horizon",
                gamepad::TextKeyboard::Steam => "Steam",
            }),
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
            Opt::NotificationDisplayTime => s.notification_display_time = step(5.0, 100.0) / 10.0,
            Opt::ChatOpacity => s.chat_opacity = v,
            Opt::TextBackgroundOpacity => s.text_background_opacity = v,
            Opt::ChatScale => s.chat_scale = v,
            Opt::ChatWidth => s.chat_width = v,
            Opt::ChatHeightFocused => s.chat_height_focused = v,
            Opt::ChatHeightUnfocused => s.chat_height_unfocused = v,
            Opt::ChatLineSpacing => s.chat_line_spacing = v,
            Opt::PadLookSensitivity => s.controller.look_sensitivity = v,
            Opt::PadMoveDeadzone => s.controller.move_deadzone = step(0.0, 50.0) / 100.0,
            Opt::PadLookDeadzone => s.controller.look_deadzone = step(0.0, 50.0) / 100.0,
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
    pub fn cycle(self, s: &mut Settings, back: bool, framebuffer: [i32; 2]) {
        let next = |n: u8, count: u8| {
            if back {
                (n + count - 1) % count
            } else {
                (n + 1) % count
            }
        };
        match self {
            Opt::GuiScale => {
                let max = crate::window::calculate_scale(framebuffer, 0, s.force_unicode) as u32;
                let value = s.gui_scale.min(max);
                s.gui_scale = if back {
                    if value == 0 { max } else { value - 1 }
                } else if value == max {
                    0
                } else {
                    value + 1
                };
            }
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
            Opt::ShowSubtitles => s.show_subtitles = !s.show_subtitles,
            Opt::ForceUnicode => s.force_unicode = !s.force_unicode,
            Opt::JapaneseGlyphs => s.japanese_glyphs = !s.japanese_glyphs,
            Opt::PadEnabled => s.controller.enabled = !s.controller.enabled,
            Opt::PadVibration => s.controller.vibration = !s.controller.vibration,
            Opt::PadInvertX => s.controller.invert_x = !s.controller.invert_x,
            Opt::PadInvertY => s.controller.invert_y = !s.controller.invert_y,
            Opt::PadToggleSneak => s.controller.toggle_sneak = !s.controller.toggle_sneak,
            Opt::PadGlyphs => {
                let i = GlyphStyle::ALL
                    .iter()
                    .position(|g| *g == s.controller.glyphs)
                    .unwrap_or(0) as u8;
                s.controller.glyphs = GlyphStyle::ALL[usize::from(next(i, 4))];
            }
            Opt::PadMenuNav => {
                s.controller.menu_nav = match s.controller.menu_nav {
                    gamepad::MenuNav::Focus => gamepad::MenuNav::Pointer,
                    gamepad::MenuNav::Pointer => gamepad::MenuNav::Focus,
                }
            }
            Opt::PadKeyboard => {
                let all = gamepad::TextKeyboard::ALL;
                let i = all
                    .iter()
                    .position(|k| *k == s.controller.keyboard)
                    .unwrap_or(0) as u8;
                s.controller.keyboard = all[usize::from(next(i, 3))];
            }
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
    /// Horizon's graphics settings (`graphics_ui`).
    Graphics,
    GraphicsAdvanced,
    /// Horizon's controller settings and button binds.
    Controller,
    PadBinds,
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
    /// Controller binds: wait for a button, reset one, reset all.
    PadKey(&'static str),
    ResetPadKey(&'static str),
    ResetPadKeys,
    /// Back to Horizon's own menus (Horizon only, see `list_rows`).
    HorizonMenus,
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
    /// An accepted button press or the release of a slider.
    ButtonClick,
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
    pub(crate) dragging: Option<Opt>,
    scrollbar_drag_y: Option<f32>,
    /// Horizon style: the selected sidebar section, the one last drawn and
    /// the rows' scroll in dp (`options_horizon`).
    pub section: usize,
    pub(crate) section_shown: Option<usize>,
    pub(crate) hscroll: f32,
    /// The slider being dragged on the Horizon options shell.
    pub(crate) hdragging: Option<crate::graphics_ui::Item>,
    /// Key Binds: the mapping waiting for a key.
    pub selected_key: Option<&'static str>,
    /// Controller Binds: the action waiting for a pad button.
    pub selected_pad_action: Option<&'static str>,
    /// The glyphs the binds list shows (the setting, else the pad's family).
    pub pad_style: GlyphStyle,
    pub languages: Vec<LanguageInfo>,
    language_choice: Option<String>,
    language_search: ui::EditBox,
    /// Packs: the packs found and the selection the screen opens with.
    pub pack_infos: Vec<crate::pack_screen::PackInfo>,
    pub pack_selection: Vec<String>,
    /// The open pack screen, made at its first frame.
    pub pack_screen: Option<crate::pack_screen::PackScreen>,
    /// When the pack screen's search box took the focus (its cursor blink).
    pack_focus: std::time::Instant,
    pub in_world: bool,
    /// Scroll the selected language into the middle on the next layout.
    center_language: bool,
    pub graphics: crate::graphics_ui::GraphicsUi,
}
impl Default for OptionsUi {
    fn default() -> Self {
        let mut search = ui::EditBox::new(0.0, 0.0, 200.0, 15.0);
        search.max_length = 32;
        Self {
            stack: vec![Page::Main],
            scroll: 0.0,
            dragging: None,
            scrollbar_drag_y: None,
            section: 0,
            section_shown: None,
            hscroll: 0.0,
            hdragging: None,
            selected_key: None,
            selected_pad_action: None,
            pad_style: GlyphStyle::Xbox,
            languages: Vec::new(),
            language_choice: None,
            language_search: search,
            pack_infos: Vec::new(),
            pack_selection: Vec::new(),
            pack_screen: None,
            pack_focus: std::time::Instant::now(),
            in_world: false,
            center_language: false,
            graphics: Default::default(),
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
        self.open_at(in_world, Page::Main);
    }
    /// Open a page whose Done/Escape returns directly to the calling screen.
    pub fn open_at(&mut self, in_world: bool, page: Page) {
        self.stack.clear();
        self.push(page);
        self.in_world = in_world;
        self.selected_key = None;
    }
    /// Open one of vanilla's pages over the current one.
    pub(crate) fn open_page(&mut self, page: Page) {
        self.push(page);
    }
    fn push(&mut self, page: Page) {
        self.stack.push(page);
        self.scroll = 0.0;
        self.scrollbar_drag_y = None;
        if page == Page::Language {
            self.language_choice = None;
            self.language_search.set_value("");
            self.center_language = true;
        }
        if page == Page::Packs {
            self.pack_screen = None;
        }
        if matches!(page, Page::Graphics | Page::GraphicsAdvanced) {
            self.graphics.reset();
        }
    }
    /// The graphics screen level of `page`, if it is one.
    fn graphics_level(page: Page) -> Option<crate::graphics_ui::Level> {
        match page {
            Page::Graphics => Some(crate::graphics_ui::Level::Simple),
            Page::GraphicsAdvanced => Some(crate::graphics_ui::Level::Advanced),
            _ => None,
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
            Page::Graphics | Page::GraphicsAdvanced => "options.videoTitle",
            Page::Controller => "Controller Settings",
            Page::PadBinds => "Controller Binds",
        }
    }
    fn scrollbar_x(&self, width: f32) -> f32 {
        (width / 2.0).floor()
            + match self.page() {
                Page::KeyBinds | Page::PadBinds => 178.0,
                Page::Language => 118.0,
                _ => 163.0,
            }
    }
    fn list_rows(&self, s: &Settings) -> Vec<Row> {
        match self.page() {
            Page::Video => {
                // Horizon's graphics screens, Simple and Advanced.
                let mut rows = vec![Row::Buttons(
                    button(
                        0.0,
                        0.0,
                        150.0,
                        Text::literal("Horizon Graphics..."),
                        Act::Push(Page::Graphics),
                    ),
                    Some(button(
                        0.0,
                        0.0,
                        150.0,
                        Text::literal("Advanced Graphics..."),
                        Act::Push(Page::GraphicsAdvanced),
                    )),
                )];
                // The way back from Classic menus. Horizon-only like the row
                // above, here rather than on the main Options screen so that
                // one keeps vanilla's exact layout; vanilla's interface
                // settings (GUI Scale) are on this page too.
                if !crate::ui_theme::horizon() {
                    rows.push(Row::Buttons(
                        button(
                            0.0,
                            0.0,
                            310.0,
                            Text::literal("Menu Style: Classic"),
                            Act::HorizonMenus,
                        ),
                        None,
                    ));
                }
                rows.push(Row::Header("options.video.display.header"));
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
                rows.extend(pairs(&[Opt::ShowSubtitles]));
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
                // Horizon's controller page, after vanilla's rows.
                rows.push(Row::Buttons(
                    button(
                        0.0,
                        0.0,
                        310.0,
                        Text::literal("Controller Settings..."),
                        Act::Push(Page::Controller),
                    ),
                    None,
                ));
                rows
            }
            Page::Controller => {
                let mut rows = vec![Row::Buttons(
                    button(
                        0.0,
                        0.0,
                        310.0,
                        Text::literal("Controller Binds..."),
                        Act::Push(Page::PadBinds),
                    ),
                    None,
                )];
                rows.extend(pairs(&[
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
                Opt::ShowSubtitles,
                Opt::HighContrast,
                Opt::MenuBlur,
                Opt::TextBackgroundOpacity,
                Opt::ChatOpacity,
                Opt::ChatLineSpacing,
                Opt::NotificationDisplayTime,
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
    fn layout(&mut self, s: &Settings, w: f32, h: f32) -> (Vec<Widget>, f32, f32, f32) {
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
                        let label =
                            Text::translate(&format!("key.category.minecraft.{category}"), &[])
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
            Page::PadBinds => {
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
                let row_left = cx - 170.0;
                let scrollbar_x = row_left + 340.0 + 6.0 + 2.0;
                let list_start = out.len();
                let mut y = top + 2.0 - self.scroll;
                let pad = &s.controller;
                for (action, _) in gamepad::ACTIONS {
                    out.push(Widget {
                        x: row_left + 2.0,
                        y: y + 6.0,
                        w: 200.0,
                        h: 9.0,
                        kind: WKind::Text {
                            text: tr(gamepad::action_caption(action)),
                            color: u32::MAX,
                            centered: false,
                        },
                        list: false,
                    });
                    let mut message = match pad.binding(action) {
                        Some(b) => {
                            let [r, g, bl] = b.color(self.pad_style);
                            Text::literal(b.label(self.pad_style))
                                .with_color(u32::from(r) << 16 | u32::from(g) << 8 | u32::from(bl))
                        }
                        None => tr("key.keyboard.unknown"),
                    };
                    if pad.conflicts(action) {
                        message = Text::literal("[ ")
                            .append(message)
                            .append(" ]")
                            .with_color(text::YELLOW);
                    }
                    if self.selected_pad_action == Some(*action) {
                        message = Text::literal("> ")
                            .append(message.with_style(&Style {
                                underlined: Some(true),
                                ..Style::default()
                            }))
                            .append(" <")
                            .with_color(text::YELLOW);
                    }
                    let reset_x = scrollbar_x - 50.0 - 10.0;
                    let change_x = reset_x - 5.0 - 75.0;
                    out.push(button(change_x, y, 75.0, message, Act::PadKey(action)));
                    let reset = button(
                        reset_x,
                        y,
                        50.0,
                        tr("controls.reset"),
                        Act::ResetPadKey(action),
                    );
                    out.push(if pad.is_default(action) {
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
                    Act::ResetPadKeys,
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
            // Drawn by `pack_screen`.
            Page::Packs => {}
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
        // Layout may change the offset (language centering), or the list may
        // shrink after a resize/search. Correct the rows in this same frame.
        let previous_scroll = self.scroll;
        self.clamp_scroll(top, bottom, content);
        for widget in &mut out {
            if widget.list {
                widget.y += previous_scroll - self.scroll;
            }
        }
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
        if crate::ui_theme::horizon() && self.page() == Page::Main {
            self.draw_horizon(s, batch, atlas, cursor);
            return;
        }
        if let Some(level) = Self::graphics_level(self.page()) {
            self.graphics
                .draw(level, s, batch, atlas, cursor, self.in_world);
            return;
        }
        if self.page() == Page::Packs {
            ui::menu_background(batch, atlas, 0.0, 0.0, w, h, self.in_world);
            let blink = self.pack_focus.elapsed().as_millis() as u64;
            let m = crate::screen_draw::Measure(&atlas.font);
            let mouse = (cursor.0.floor() as i32, cursor.1.floor() as i32);
            let draws = self.packs(atlas, (w, h)).draw(&m, mouse, blink);
            crate::screen_draw::paint(batch, atlas, &draws);
            return;
        }
        let (widgets, top, bottom, content) = self.layout(s, w, h);
        let in_world = self.in_world;
        ui::menu_background(batch, atlas, 0.0, 0.0, w, h, in_world);
        let list = bottom > top;
        if list {
            ui::list_background(batch, atlas, top, bottom, in_world);
        }
        let visible = |wd: &Widget| !wd.list || (wd.y < bottom && wd.y + wd.h > top);
        let mut tooltip = None;
        for wd in &widgets {
            if !visible(wd) {
                continue;
            }
            let start = batch.vertices.len();
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
                    crate::widgets::target_as([wd.x, wd.y, wd.w, wd.h], crate::widgets::Kind::Row);
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
            }
            if wd.list && (wd.y < top || wd.y + wd.h > bottom) {
                batch.clip_since(start, 0.0, top, w, bottom);
            }
        }
        if list {
            ui::separators(batch, atlas, top, bottom, in_world);
            let sx = self.scrollbar_x(w);
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
        framebuffer: [i32; 2],
        drag: bool,
        right: bool,
    ) -> Vec<Event> {
        if crate::ui_theme::horizon() && self.page() == Page::Main {
            return self.click_horizon(s, atlas, x, y, framebuffer, drag, right);
        }
        if let Some(level) = Self::graphics_level(self.page()) {
            use crate::graphics_ui::Action;
            let mut events = Vec::new();
            for action in self
                .graphics
                .click(level, s, (x, y), size, framebuffer, drag, right)
            {
                match action {
                    Action::ButtonClick => events.push(Event::ButtonClick),
                    Action::Changed => events.push(Event::Changed),
                    Action::OpenAdvanced => self.push(Page::GraphicsAdvanced),
                    Action::OpenSimple => {
                        if self.stack.len() >= 2
                            && self.stack[self.stack.len() - 2] == Page::Graphics
                        {
                            events.extend(self.back(s));
                        } else {
                            self.stack.pop();
                            self.push(Page::Graphics);
                        }
                    }
                    Action::Done => events.extend(self.back(s)),
                }
            }
            return events;
        }
        if self.page() == Page::Packs {
            let mut events = Vec::new();
            if drag {
                self.packs(atlas, size).drag(y as f64);
                return events;
            }
            if right {
                return events;
            }
            let m = crate::screen_draw::Measure(&atlas.font);
            let p = self.packs(atlas, size);
            if p.button_at(&m, x as f64, y as f64) {
                events.push(Event::ButtonClick);
            }
            let before = p.search_focused();
            let action = p.click(&m, x as f64, y as f64);
            if !before && p.search_focused() {
                self.pack_focus = std::time::Instant::now();
            }
            match action {
                crate::pack_screen::Action::OpenFolder => events.push(Event::OpenPackFolder),
                crate::pack_screen::Action::Done => events.extend(self.back(s)),
                crate::pack_screen::Action::None => {}
            }
            return events;
        }
        let (widgets, top, bottom, content) = self.layout(s, size.0, size.1);
        let mut events = Vec::new();
        if drag {
            if let Some(previous_y) = self.scrollbar_drag_y {
                if let Some((_, thumb)) = ui::scrollbar_thumb(top, bottom, content, self.scroll) {
                    let max = content - (bottom - top);
                    self.scroll = if y < top {
                        0.0
                    } else if y > bottom {
                        max
                    } else {
                        self.scroll + (y - previous_y) * max / (bottom - top - thumb)
                    };
                    self.clamp_scroll(top, bottom, content);
                    self.scrollbar_drag_y = Some(y);
                } else {
                    self.scrollbar_drag_y = None;
                }
                return events;
            }
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
        // Controller Binds: a click cancels the wait.
        if self.selected_pad_action.take().is_some() {
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
        // AbstractWidget accepts only the primary button. Rebinding above
        // accepts other mouse buttons, but is not a widget activation.
        if right {
            return events;
        }
        if ui::scrollbar_thumb(top, bottom, content, self.scroll).is_some()
            && ui::hovered((x, y), self.scrollbar_x(size.0), top, 6.0, bottom - top)
        {
            self.scrollbar_drag_y = Some(y);
            return events;
        }
        for wd in widgets {
            if !ui::hovered((x, y), wd.x, wd.y, wd.w, wd.h) {
                continue;
            }
            if wd.list && (y < top || y >= bottom) {
                continue;
            }
            match wd.kind {
                WKind::Button {
                    active: true, act, ..
                } => {
                    events.push(Event::ButtonClick);
                    match act {
                        Act::Push(p) => self.push(p),
                        Act::Done => events.extend(self.back(s)),
                        Act::Cycle(opt) => {
                            opt.cycle(s, right, framebuffer);
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
                        Act::PadKey(action) => self.selected_pad_action = Some(action),
                        Act::ResetPadKey(action) => {
                            s.controller
                                .bind(action, gamepad::PadSettings::default_of(action));
                            events.push(Event::Changed);
                        }
                        Act::ResetPadKeys => {
                            s.controller.reset_bindings();
                            events.push(Event::Changed);
                        }
                        Act::HorizonMenus => {
                            self.horizon_menus();
                            if let Ok(dir) = std::env::current_dir() {
                                crate::ui_theme::save_preferences(&dir);
                            }
                        }
                        Act::None => {}
                    }
                }
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
                _ => {}
            }
            break;
        }
        events
    }
    /// Switch to Horizon's menus and show where the choice lives there
    /// (Options, Interface, Menu style).
    fn horizon_menus(&mut self) {
        crate::ui_theme::set_style(crate::ui_theme::MenuStyle::Horizon);
        self.stack.truncate(1);
        self.scroll = 0.0;
        self.scrollbar_drag_y = None;
        self.section = crate::options_horizon::SECTIONS
            .iter()
            .position(|section| section.name == "Interface")
            .unwrap_or(0);
        self.section_shown = None;
        self.hscroll = 0.0;
    }
    pub fn release(&mut self) -> Vec<Event> {
        self.scrollbar_drag_y = None;
        if let Some(p) = &mut self.pack_screen {
            p.release();
        }
        // AbstractSliderButton suppresses the press sound and plays once on
        // release, even if the pointer has left the widget or its value stayed.
        let slider = self.dragging.take().is_some();
        self.hdragging = None;
        let graphics_slider = self.graphics.release();
        if slider || graphics_slider {
            vec![Event::ButtonClick]
        } else {
            Vec::new()
        }
    }
    pub fn scroll(&mut self, s: &Settings, size: (f32, f32), amount: f32) {
        let (_, top, bottom, content) = self.layout(s, size.0, size.1);
        self.scroll -= amount * 18.0;
        self.clamp_scroll(top, bottom, content);
    }
    /// The mouse wheel at the cursor.
    pub fn scroll_at(&mut self, s: &Settings, size: (f32, f32), cursor: (f32, f32), amount: f32) {
        if crate::ui_theme::horizon() && self.page() == Page::Main {
            self.scroll_horizon(amount);
            return;
        }
        if let Some(level) = Self::graphics_level(self.page()) {
            self.graphics.scroll(level, size.1, amount);
            return;
        }
        match &mut self.pack_screen {
            Some(p) if self.stack.last() == Some(&Page::Packs) => {
                p.scroll(cursor.0 as f64, cursor.1 as f64, amount as f64)
            }
            _ => self.scroll(s, size, amount),
        }
    }
    /// The pack screen at this GUI size, made on first use.
    fn packs(&mut self, atlas: &HudAtlas, size: (f32, f32)) -> &mut crate::pack_screen::PackScreen {
        let m = crate::screen_draw::Measure(&atlas.font);
        let (w, h) = (size.0 as i32, size.1 as i32);
        match &mut self.pack_screen {
            Some(p) => {
                if p.width != w || p.height != h {
                    p.resize(&m, w, h);
                }
            }
            None => {
                self.pack_focus = std::time::Instant::now();
                self.pack_screen = Some(crate::pack_screen::PackScreen::new(
                    &m,
                    w,
                    h,
                    self.in_world,
                    self.pack_infos.clone(),
                    &self.pack_selection,
                ));
            }
        }
        self.pack_screen.as_mut().expect("made above")
    }
    /// Escape or Done: leave the current page, applying page results.
    pub fn back(&mut self, s: &mut Settings) -> Vec<Event> {
        let mut events = Vec::new();
        let page = self.stack.pop().unwrap_or(Page::Main);
        self.scroll = 0.0;
        self.scrollbar_drag_y = None;
        self.dragging = None;
        self.selected_key = None;
        self.selected_pad_action = None;
        match page {
            Page::Language => {
                if let Some(code) = self.language_choice.take()
                    && code != s.language
                {
                    s.language = code.clone();
                    events.push(Event::Language(code));
                }
            }
            Page::Packs => {
                // PackSelectionScreen.onClose: PackSelectionModel.commit.
                if let Some(p) = self.pack_screen.take()
                    && p.changed
                {
                    let ids = p.commit();
                    s.resource_packs = ids.clone();
                    events.push(Event::Packs(ids));
                }
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
    pub fn type_text(&mut self, atlas: &HudAtlas, text: &str) {
        if self.page() == Page::Packs
            && let Some(p) = &mut self.pack_screen
        {
            p.type_text(&crate::screen_draw::Measure(&atlas.font), text);
            return;
        }
        if self.page() == Page::Language {
            self.language_search.insert_text(text);
            self.scroll = 0.0;
        }
    }
    /// Ctrl+A/C/V/X in the language or pack search box; false when neither
    /// is shown with the focus.
    pub fn shortcut(
        &mut self,
        atlas: &HudAtlas,
        shortcut: crate::clipboard::EditShortcut,
        clipboard: &mut dyn crate::clipboard::Clipboard,
    ) -> bool {
        if self.selected_key.is_some() {
            return false;
        }
        if self.page() == Page::Packs
            && let Some(p) = &mut self.pack_screen
        {
            let m = crate::screen_draw::Measure(&atlas.font);
            return p.shortcut(&m, shortcut, clipboard);
        }
        if self.page() == Page::Language {
            if self.language_search.shortcut(shortcut, clipboard) {
                self.scroll = 0.0;
            }
            return true;
        }
        false
    }
    /// Keys while the options are open. Returns events; `handled` false lets
    /// Escape fall through.
    pub fn key(
        &mut self,
        s: &mut Settings,
        atlas: &HudAtlas,
        key: winit::keyboard::KeyCode,
        text: Option<&str>,
    ) -> Vec<Event> {
        use winit::keyboard::KeyCode as K;
        if self.page() == Page::Packs
            && let Some(p) = &mut self.pack_screen
        {
            let m = crate::screen_draw::Measure(&atlas.font);
            match key {
                // ConfirmScreen.keyPressed: Escape answers no.
                K::Escape if p.confirming() => {
                    p.cancel_confirm(&m);
                    return Vec::new();
                }
                K::Backspace => {
                    p.erase(&m, 1);
                    return Vec::new();
                }
                _ => {}
            }
        }
        if let Some(action) = self.selected_pad_action.take() {
            // As on Key Binds, Escape unbinds; other keys cancel.
            if key == K::Escape {
                s.controller.bind(action, None);
            }
            return vec![Event::Changed];
        }
        if let Some(name) = self.selected_key.take() {
            // KeyBindsScreen.keyPressed: Escape unbinds.
            let value = if key == K::Escape {
                "key.keyboard.unknown".to_owned()
            } else {
                keybinds::key_name(key)
                    .unwrap_or("key.keyboard.unknown")
                    .to_owned()
            };
            if name == crate::extras::ZOOM_KEY {
                s.extras.zoom_key = value;
            } else {
                s.keys.set(name, &value);
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui_theme::{MenuStyle, set_style, style};

    fn atlas() -> HudAtlas {
        HudAtlas::build(std::path::Path::new("/nonexistent/horizon-test-assets")).unwrap()
    }

    #[test]
    fn classic_video_gui_scale_cycles_on_press_without_dragging() {
        let atlas = atlas();
        let mut s = Settings::default();
        let mut ui = OptionsUi::default();
        ui.show(Page::Video);
        let size = (480.0, 480.0);
        let (widgets, _, _, _) = ui.layout(&s, size.0, size.1);
        let button = widgets
            .iter()
            .find(|wd| {
                matches!(
                    wd.kind,
                    WKind::Button {
                        active: true,
                        act: Act::Cycle(Opt::GuiScale),
                        ..
                    }
                )
            })
            .expect("GUI Scale must be a cycle button in classic Video Settings");
        let (x, y) = (button.x + 1.0, button.y + 1.0);
        assert!(Opt::GuiScale.slider_position(&s).is_none());
        assert!(
            ui.click(&mut s, &atlas, x, y, size, [854, 480], false, true)
                .is_empty()
        );
        assert_eq!(s.gui_scale, 0);
        for expected in [1, 2, 0] {
            assert_eq!(
                ui.click(&mut s, &atlas, x, y, size, [854, 480], false, false),
                vec![Event::ButtonClick, Event::Changed]
            );
            assert_eq!(s.gui_scale, expected);
            assert!(
                ui.click(&mut s, &atlas, x + 100.0, y, size, [854, 480], true, false)
                    .is_empty()
            );
            assert_eq!(s.gui_scale, expected);
            assert!(ui.release().is_empty());
        }
    }

    #[test]
    fn gui_scale_cycle_handles_reverse_and_out_of_range_saved_values() {
        for (framebuffer, force_unicode, max) in [
            ([854, 480], false, 2),
            ([1920, 1080], false, 4),
            ([1280, 720], true, 4),
            ([3840, 2160], false, 9),
            ([320, 240], false, 1),
        ] {
            let mut s = Settings {
                force_unicode,
                ..Settings::default()
            };
            for expected in (0..=max).rev() {
                Opt::GuiScale.cycle(&mut s, true, framebuffer);
                assert_eq!(s.gui_scale, expected);
            }
            for expected in (1..=max).chain([0]) {
                Opt::GuiScale.cycle(&mut s, false, framebuffer);
                assert_eq!(s.gui_scale, expected);
            }
            s.gui_scale = u32::MAX;
            s.clamp_gui_scale(framebuffer);
            assert_eq!(s.gui_scale, max);
            s.gui_scale = u32::MAX;
            Opt::GuiScale.cycle(&mut s, false, framebuffer);
            assert_eq!(s.gui_scale, 0);
            s.gui_scale = u32::MAX;
            Opt::GuiScale.cycle(&mut s, true, framebuffer);
            assert_eq!(s.gui_scale, max - 1);
        }
        let mut s = Settings {
            gui_scale: 4,
            ..Settings::default()
        };
        // A resized window supplies a new maximum on the next interaction.
        Opt::GuiScale.cycle(&mut s, true, [854, 480]);
        assert_eq!(s.gui_scale, 1);
    }

    #[test]
    fn classic_buttons_sound_but_disabled_and_nonprimary_clicks_are_silent() {
        let atlas = atlas();
        let mut s = Settings::default();
        let mut ui = OptionsUi::default();
        ui.show(Page::Mouse);
        let size = (480.0, 320.0);
        let (widgets, _, _, _) = ui.layout(&s, size.0, size.1);
        let button = widgets
            .iter()
            .find(|wd| {
                matches!(
                    wd.kind,
                    WKind::Button {
                        active: true,
                        act: Act::Cycle(_),
                        ..
                    }
                )
            })
            .unwrap();
        let (x, y) = (button.x + 1.0, button.y + 1.0);
        assert!(
            ui.click(&mut s, &atlas, x, y, size, [854, 480], false, true)
                .is_empty()
        );
        assert_eq!(
            ui.click(&mut s, &atlas, x, y, size, [854, 480], false, false),
            vec![Event::ButtonClick, Event::Changed]
        );
        assert!(ui.release().is_empty());
        assert!(
            ui.click(&mut s, &atlas, 0.0, 0.0, size, [854, 480], false, false)
                .is_empty()
        );
        ui.show(Page::KeyBinds);
        let (widgets, _, _, _) = ui.layout(&s, size.0, size.1);
        let disabled = widgets
            .iter()
            .find(|wd| matches!(wd.kind, WKind::Button { active: false, .. }))
            .unwrap();
        assert!(
            ui.click(
                &mut s,
                &atlas,
                disabled.x + 1.0,
                disabled.y + 1.0,
                size,
                [854, 480],
                false,
                false
            )
            .is_empty()
        );
    }

    #[test]
    fn classic_sliders_sound_once_on_release_including_unchanged_values() {
        let atlas = atlas();
        let size = (480.0, 320.0);
        let mut s = Settings::default();
        let mut ui = OptionsUi::default();
        ui.show(Page::Sound);
        let (widgets, _, _, _) = ui.layout(&s, size.0, size.1);
        let slider = widgets
            .iter()
            .find(|wd| {
                matches!(
                    wd.kind,
                    WKind::Slider {
                        opt: Opt::Volume(0),
                        ..
                    }
                )
            })
            .unwrap();
        let (x, y) = (slider.x + slider.w - 1.0, slider.y + 1.0);
        assert_eq!(
            ui.click(&mut s, &atlas, x, y, size, [854, 480], false, false),
            vec![Event::Changed]
        );
        assert_eq!(s.master_volume, 1.0);
        assert_eq!(ui.release(), vec![Event::ButtonClick]);
        assert!(ui.release().is_empty());
        assert_eq!(
            ui.click(&mut s, &atlas, x, y, size, [854, 480], false, false),
            vec![Event::Changed]
        );
        for cx in [-100.0, 250.0, 1000.0] {
            assert_eq!(
                ui.click(&mut s, &atlas, cx, -100.0, size, [854, 480], true, false),
                vec![Event::Changed]
            );
        }
        assert_eq!(ui.release(), vec![Event::ButtonClick]);
        assert!(
            ui.click(&mut s, &atlas, x, y, size, [854, 480], true, false)
                .is_empty()
        );
    }

    #[test]
    fn classic_language_rows_and_text_fields_are_silent() {
        let atlas = atlas();
        let mut s = Settings::default();
        let mut ui = OptionsUi {
            languages: languages(),
            ..Default::default()
        };
        ui.show(Page::Language);
        let size = (480.0, 320.0);
        let (widgets, top, bottom, _) = ui.layout(&s, size.0, size.1);
        let row = widgets
            .iter()
            .find(|wd| {
                matches!(wd.kind, WKind::LanguageRow { .. }) && wd.y >= top && wd.y + wd.h <= bottom
            })
            .unwrap();
        assert!(
            ui.click(
                &mut s,
                &atlas,
                row.x + 1.0,
                row.y + 1.0,
                size,
                [854, 480],
                false,
                false
            )
            .is_empty()
        );
        assert!(ui.release().is_empty());
        let search = ui.language_search.clone();
        assert!(
            ui.click(
                &mut s,
                &atlas,
                search.x + 1.0,
                search.y + 1.0,
                size,
                [854, 480],
                false,
                false
            )
            .is_empty()
        );
    }

    fn languages() -> Vec<LanguageInfo> {
        (0..30)
            .map(|i| LanguageInfo {
                code: format!("language_{i}"),
                name: format!("Language {i}"),
                region: String::new(),
                bidirectional: false,
            })
            .collect()
    }

    #[test]
    fn classic_scroll_stops_at_both_limits_before_layout() {
        let s = Settings::default();
        let size = (480.0, 120.0);
        for page in [Page::KeyBinds, Page::Chat, Page::Sound, Page::Language] {
            let mut ui = OptionsUi {
                languages: languages(),
                ..Default::default()
            };
            ui.show(page);
            let (_, top, bottom, content) = ui.layout(&s, size.0, size.1);
            let max = content - (bottom - top);
            assert!(max > 4.5, "{page:?}");
            for _ in 0..3 {
                ui.scroll_at(&s, size, (240.0, 60.0), 1000.0);
                assert_eq!(ui.scroll, 0.0, "{page:?}");
            }
            for _ in 0..3 {
                ui.scroll_at(&s, size, (240.0, 60.0), -1000.0);
                assert_eq!(ui.scroll, max, "{page:?}");
            }
            ui.scroll_at(&s, size, (240.0, 60.0), 0.25);
            assert_eq!(ui.scroll, max - 4.5, "{page:?}");
            ui.scroll_at(&s, size, (240.0, 60.0), 1000.0);
            ui.scroll_at(&s, size, (240.0, 60.0), -0.25);
            assert_eq!(ui.scroll, 4.5, "{page:?}");
        }
    }

    #[test]
    fn partially_visible_keybind_rows_are_clipped_and_clickable() {
        let mut atlas = atlas();
        let button = crate::hud::Rect {
            u0: 0.25,
            v0: 0.25,
            u1: 0.5,
            v1: 0.5,
            w: 20,
            h: 20,
        };
        for name in ["button", "button_disabled", "button_highlighted"] {
            atlas
                .sprites
                .insert(format!("gui/sprites/widget/{name}"), button);
        }
        for above in [true, false] {
            let mut s = Settings::default();
            let mut ui = OptionsUi::default();
            ui.show(Page::KeyBinds);
            let mut batch = HudBatch::new([480, 240]);
            let size = (batch.width, batch.height);
            let (widgets, top, bottom, _) = ui.layout(&s, size.0, size.1);
            let row = widgets
                .iter()
                .filter(|w| {
                    matches!(
                        w.kind,
                        WKind::Button {
                            act: Act::Key(_),
                            ..
                        }
                    )
                })
                .nth(10)
                .unwrap();
            let WKind::Button {
                act: Act::Key(key), ..
            } = row.kind
            else {
                unreachable!()
            };
            let row_y = if above { top - 5.0 } else { bottom - 16.0 };
            ui.scroll = row.y - row_y;
            let (x, width, height) = (row.x, row.w, row.h);
            ui.draw(&s, &mut batch, &atlas, (-1.0, -1.0));
            let vertices: Vec<_> = batch
                .vertices
                .iter()
                .filter(|v| {
                    (0.25..=0.5).contains(&v.uv[0])
                        && (0.25..=0.5).contains(&v.uv[1])
                        && v.pos[0] >= x
                        && v.pos[0] <= x + width
                        && v.pos[1] >= row_y
                        && v.pos[1] <= row_y + height
                })
                .collect();
            assert!(!vertices.is_empty());
            let lo = vertices
                .iter()
                .map(|v| v.pos[1])
                .fold(f32::INFINITY, f32::min);
            let hi = vertices
                .iter()
                .map(|v| v.pos[1])
                .fold(f32::NEG_INFINITY, f32::max);
            assert_eq!(lo, row_y.max(top));
            assert_eq!(hi, (row_y + height).min(bottom));
            let hidden_y = if above {
                row_y + 1.0
            } else {
                row_y + height - 1.0
            };
            assert!(
                ui.click(
                    &mut s,
                    &atlas,
                    x + 1.0,
                    hidden_y,
                    size,
                    [480, 240],
                    false,
                    false
                )
                .is_empty()
            );
            assert_eq!(ui.selected_key, None);
            assert_eq!(
                ui.click(
                    &mut s,
                    &atlas,
                    x + 1.0,
                    (lo + hi) / 2.0,
                    size,
                    [480, 240],
                    false,
                    false
                ),
                vec![Event::ButtonClick]
            );
            assert_eq!(ui.selected_key, Some(key));
        }
    }

    #[test]
    fn classic_scrollbar_drag_is_bounded_silent_and_stops_on_release() {
        let atlas = atlas();
        let size = (480.0, 160.0);
        for page in [Page::KeyBinds, Page::PadBinds, Page::Language, Page::Chat] {
            let mut s = Settings::default();
            let mut ui = OptionsUi {
                languages: languages(),
                ..Default::default()
            };
            ui.show(page);
            let (_, top, bottom, content) = ui.layout(&s, size.0, size.1);
            let max = content - (bottom - top);
            assert!(max > 0.0, "{page:?}");
            let x = ui.scrollbar_x(size.0) + 2.0;
            let (_, thumb) = ui::scrollbar_thumb(top, bottom, content, ui.scroll).unwrap();
            let y = top + thumb / 2.0;
            assert!(
                ui.click(&mut s, &atlas, x, y, size, [960, 480], false, true)
                    .is_empty()
            );
            assert_eq!(ui.scrollbar_drag_y, None);
            assert!(
                ui.click(&mut s, &atlas, x, y, size, [960, 480], false, false)
                    .is_empty()
            );
            assert_eq!(ui.scrollbar_drag_y, Some(y));
            let delta = 5.0;
            assert!(
                ui.click(
                    &mut s,
                    &atlas,
                    x - 100.0,
                    y + delta,
                    size,
                    [960, 480],
                    true,
                    false
                )
                .is_empty()
            );
            assert!((ui.scroll - delta * max / (bottom - top - thumb)).abs() < 0.001);
            for (y, expected) in [(bottom + 100.0, max), (top - 100.0, 0.0)] {
                assert!(
                    ui.click(&mut s, &atlas, x, y, size, [960, 480], true, false)
                        .is_empty()
                );
                assert!((ui.scroll - expected).abs() < 0.001, "{page:?}");
            }
            assert!(ui.release().is_empty());
            assert_eq!(ui.scrollbar_drag_y, None);
            assert!(
                ui.click(
                    &mut s,
                    &atlas,
                    x,
                    bottom + 100.0,
                    size,
                    [960, 480],
                    true,
                    false
                )
                .is_empty()
            );
            assert_eq!(ui.scroll, 0.0);
            assert_eq!(s.keys, Settings::default().keys);
            ui.click(&mut s, &atlas, x, top + 1.0, size, [960, 480], false, false);
            assert!(ui.scrollbar_drag_y.is_some());
            ui.show(Page::Mouse);
            assert_eq!(ui.scrollbar_drag_y, None);
        }
    }

    #[test]
    fn key_rebinding_takes_precedence_over_scrollbar_dragging() {
        let atlas = atlas();
        let mut s = Settings::default();
        let mut ui = OptionsUi::default();
        ui.show(Page::KeyBinds);
        ui.selected_key = Some("key.jump");
        let x = ui.scrollbar_x(480.0) + 2.0;
        assert_eq!(
            ui.click(
                &mut s,
                &atlas,
                x,
                50.0,
                (480.0, 240.0),
                [960, 480],
                false,
                false
            ),
            vec![Event::Changed]
        );
        assert_eq!(s.keys.get("key.jump"), "key.mouse.left");
        assert_eq!(ui.scrollbar_drag_y, None);
        assert!(ui.release().is_empty());
    }

    #[test]
    fn classic_pages_without_overflow_do_not_scroll() {
        let s = Settings::default();
        for page in [Page::Main, Page::Font, Page::Mouse] {
            let mut ui = OptionsUi::default();
            ui.show(page);
            for amount in [-1000.0, 1000.0, -0.25] {
                ui.scroll(&s, (480.0, 720.0), amount);
                assert_eq!(ui.scroll, 0.0, "{page:?}");
            }
        }
    }

    #[test]
    fn classic_resize_positions_rows_at_the_clamped_offset() {
        let s = Settings::default();
        let mut ui = OptionsUi::default();
        ui.show(Page::KeyBinds);
        ui.scroll(&s, (480.0, 120.0), -1000.0);
        assert!(ui.scroll > 0.0);
        let (widgets, _, _, _) = ui.layout(&s, 480.0, 4000.0);
        assert_eq!(ui.scroll, 0.0);
        let (expected, _, _, _) = ui.layout(&s, 480.0, 4000.0);
        assert_eq!(
            widgets.iter().map(|w| w.y).collect::<Vec<_>>(),
            expected.iter().map(|w| w.y).collect::<Vec<_>>()
        );
    }

    #[test]
    fn language_centering_and_filtering_use_bounded_row_positions() {
        let s = Settings {
            language: "language_29".into(),
            ..Default::default()
        };
        let mut ui = OptionsUi {
            languages: languages(),
            ..Default::default()
        };
        ui.show(Page::Language);
        let (widgets, top, bottom, content) = ui.layout(&s, 480.0, 240.0);
        assert_eq!(ui.scroll, content - (bottom - top));
        let (expected, _, _, _) = ui.layout(&s, 480.0, 240.0);
        assert_eq!(
            widgets.iter().map(|w| w.y).collect::<Vec<_>>(),
            expected.iter().map(|w| w.y).collect::<Vec<_>>()
        );
        ui.language_search.set_value("language_29");
        let (widgets, top, _, _) = ui.layout(&s, 480.0, 240.0);
        assert_eq!(ui.scroll, 0.0);
        assert_eq!(widgets.iter().find(|w| w.list).unwrap().y, top + 2.0);
    }

    #[test]
    fn title_shortcuts_initialize_pages_and_return_directly_to_the_title() {
        let mut ui = OptionsUi::default();
        let mut s = Settings::default();
        for page in [Page::Accessibility, Page::Language] {
            ui.language_choice = Some("de_de".into());
            ui.language_search.set_value("old query");
            ui.open_at(false, page);
            assert_eq!(ui.page(), page);
            assert!(!ui.in_world);
            if page == Page::Language {
                assert_eq!(ui.language_choice, None);
                assert!(ui.language_search.value.is_empty());
                assert!(ui.center_language);
            }
            assert!(ui.back(&mut s).contains(&Event::Close));
            assert_eq!(s.language, "en_us");
            ui.open(false);
            ui.show(page);
            assert!(!ui.back(&mut s).contains(&Event::Close));
            assert_eq!(ui.page(), Page::Main);
        }
        ui.open_at(false, Page::Language);
        ui.language_choice = Some("de_de".into());
        let events = ui.back(&mut s);
        assert!(events.contains(&Event::Language("de_de".into())));
        assert!(events.contains(&Event::Close));
        assert_eq!(s.language, "de_de");
    }

    #[test]
    fn classic_video_settings_lead_back_to_horizon_menus() {
        let s = Settings::default();
        let mut ui = OptionsUi::default();
        ui.push(Page::Video);
        let way_back = |ui: &OptionsUi| {
            ui.list_rows(&s).iter().any(|row| {
                matches!(row, Row::Buttons(w, _)
                    if matches!(&w.kind, WKind::Button { act: Act::HorizonMenus, .. }))
            })
        };
        // Horizon's own menus have the choice on their Interface page.
        assert!(!way_back(&ui));
        set_style(MenuStyle::Classic);
        let classic = way_back(&ui);
        ui.horizon_menus();
        assert!(classic, "Classic Video Settings offer Menu Style");
        assert_eq!(style(), MenuStyle::Horizon);
        assert_eq!(ui.stack, vec![Page::Main]);
        assert_eq!(
            crate::options_horizon::SECTIONS[ui.section].name,
            "Interface"
        );
    }
}
