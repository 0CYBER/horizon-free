//! Player settings persisted in `options.txt` with vanilla's keys and value
//! encodings (Options.processOptions); lines Horizon does not know are kept.
//! The option screens live in `options_ui`.
use crate::keybinds::KeyBinds;
use std::{collections::BTreeMap, fs, path::Path};

/// SoundSource names in vanilla order.
pub const SOUND_SOURCES: [&str; 11] = [
    "master", "music", "record", "weather", "block", "hostile", "neutral", "player", "ambient",
    "voice", "ui",
];

/// Per-source volumes for the mixer (soundCategory_* except master).
static SOURCE_VOLUMES: std::sync::RwLock<[f32; 11]> = std::sync::RwLock::new([1.0; 11]);
pub fn set_source_volumes(volumes: [f32; 11]) {
    *SOURCE_VOLUMES.write().unwrap_or_else(|e| e.into_inner()) = volumes;
}
/// The SoundSource an event plays on. Vanilla callers pick the source; this
/// follows their conventions by event family.
pub fn sound_source(event: &str) -> usize {
    let e = event.trim_start_matches("minecraft:");
    let index = |name: &str| SOUND_SOURCES.iter().position(|n| *n == name).unwrap_or(0);
    const HOSTILE: [&str; 24] = [
        "zombie",
        "skeleton",
        "creeper",
        "spider",
        "cave_spider",
        "enderman",
        "witch",
        "slime",
        "magma_cube",
        "blaze",
        "ghast",
        "drowned",
        "husk",
        "stray",
        "phantom",
        "pillager",
        "vindicator",
        "evoker",
        "ravager",
        "guardian",
        "elder_guardian",
        "silverfish",
        "zombified_piglin",
        "wither_skeleton",
    ];
    if e.starts_with("music_disc.") {
        index("record")
    } else if e.starts_with("music.") {
        index("music")
    } else if e.starts_with("weather.") || e.starts_with("entity.lightning_bolt") {
        index("weather")
    } else if e.starts_with("block.") {
        index("block")
    } else if e.starts_with("ambient.") {
        index("ambient")
    } else if e.starts_with("ui.") {
        index("ui")
    } else if e.starts_with("entity.player.")
        || e.starts_with("item.")
        || e.starts_with("entity.item.")
        || e.starts_with("entity.experience_orb")
        || e.starts_with("entity.arrow")
    {
        index("player")
    } else if let Some(rest) = e.strip_prefix("entity.") {
        let mob = rest.split('.').next().unwrap_or("");
        if HOSTILE.contains(&mob) {
            index("hostile")
        } else {
            index("neutral")
        }
    } else {
        index("master")
    }
}
/// Volume multiplier of an event's source (master applied separately).
pub fn event_volume(event: &str) -> f32 {
    let i = sound_source(event);
    if i == 0 {
        1.0
    } else {
        SOURCE_VOLUMES.read().unwrap_or_else(|e| e.into_inner())[i]
    }
}

#[derive(Clone, Debug)]
pub struct Settings {
    /// Degrees, 30..=110 (options.txt stores (fov - 70) / 40).
    pub fov: f32,
    /// Chunks, 2..=32.
    pub render_distance: i32,
    /// Chunks, 5..=32.
    pub simulation_distance: i32,
    /// 0..1, vanilla default 0.5.
    pub sensitivity: f32,
    /// 0..1 (soundCategory_master).
    pub master_volume: f32,
    /// Volumes of every other sound source, indexed like SOUND_SOURCES.
    pub volumes: [f32; 11],
    /// 0 = auto.
    pub gui_scale: u32,
    pub view_bobbing: bool,
    /// Brightness 0..1 (gamma).
    pub gamma: f32,
    /// 10..=260 (260 = unlimited).
    pub max_fps: u32,
    pub vsync: bool,
    pub fullscreen: bool,
    /// Smooth lighting (ao).
    pub smooth_lighting: bool,
    /// "false", "fast" or "true" (renderClouds).
    pub clouds: String,
    pub cloud_range: i32,
    /// 0 all, 1 decreased, 2 minimal.
    pub particles: u8,
    pub entity_distance: f32,
    pub entity_shadows: bool,
    pub biome_blend: i32,
    /// "fast", "fancy", "fabulous" or "custom".
    pub graphics_preset: String,
    pub mipmap_levels: i32,
    pub weather_radius: i32,
    pub chunk_fade: f32,
    pub vignette: bool,
    pub autosave_indicator: bool,
    pub cutout_leaves: bool,
    pub improved_transparency: bool,
    /// 0 off, 1 crosshair, 2 hotbar.
    pub attack_indicator: u8,
    pub inactivity_fps_limit: String,
    pub prioritize_chunk_updates: u8,
    pub menu_blur: i32,
    pub invert_y: bool,
    pub mouse_wheel_sensitivity: f32,
    pub discrete_scroll: bool,
    pub toggle_crouch: bool,
    pub toggle_sprint: bool,
    pub auto_jump: bool,
    pub language: String,
    pub force_unicode: bool,
    pub japanese_glyphs: bool,
    pub chat_visibility: u8,
    pub chat_colors: bool,
    pub chat_links: bool,
    pub chat_opacity: f32,
    pub text_background_opacity: f32,
    pub chat_scale: f32,
    pub chat_width: f32,
    pub chat_height_focused: f32,
    pub chat_height_unfocused: f32,
    pub chat_line_spacing: f32,
    pub auto_suggestions: bool,
    pub reduced_debug_info: bool,
    pub high_contrast: bool,
    /// `showSubtitles`: the closed-caption overlay.
    pub show_subtitles: bool,
    /// `notificationDisplayTime`: a multiplier for subtitle and toast times.
    pub notification_display_time: f32,
    pub main_hand_left: bool,
    /// Enabled resource packs, bottom first (resourcePacks).
    pub resource_packs: Vec<String>,
    pub keys: KeyBinds,
    /// Lines Horizon does not interpret, kept verbatim.
    pub extra: BTreeMap<String, String>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            fov: 70.0,
            render_distance: 12,
            simulation_distance: 12,
            sensitivity: 0.5,
            master_volume: 1.0,
            volumes: [1.0; 11],
            gui_scale: 0,
            view_bobbing: true,
            gamma: 0.5,
            // Vanilla defaults to 120; Horizon runs unlimited unless capped.
            max_fps: 260,
            vsync: true,
            fullscreen: false,
            smooth_lighting: true,
            clouds: "true".into(),
            cloud_range: 128,
            particles: 0,
            entity_distance: 1.0,
            entity_shadows: true,
            biome_blend: 2,
            graphics_preset: "fancy".into(),
            mipmap_levels: 4,
            weather_radius: 10,
            chunk_fade: 0.75,
            vignette: true,
            autosave_indicator: true,
            cutout_leaves: true,
            improved_transparency: false,
            attack_indicator: 1,
            inactivity_fps_limit: "afk".into(),
            prioritize_chunk_updates: 0,
            menu_blur: 5,
            invert_y: false,
            mouse_wheel_sensitivity: 1.0,
            discrete_scroll: false,
            toggle_crouch: false,
            toggle_sprint: false,
            auto_jump: false,
            language: "en_us".into(),
            force_unicode: false,
            japanese_glyphs: false,
            chat_visibility: 0,
            chat_colors: true,
            chat_links: true,
            chat_opacity: 1.0,
            text_background_opacity: 0.5,
            chat_scale: 1.0,
            chat_width: 1.0,
            chat_height_focused: 1.0,
            chat_height_unfocused: 70.0 / 160.0,
            chat_line_spacing: 0.0,
            auto_suggestions: true,
            reduced_debug_info: false,
            high_contrast: false,
            show_subtitles: false,
            notification_display_time: 1.0,
            main_hand_left: false,
            resource_packs: vec!["vanilla".into()],
            keys: KeyBinds::default(),
            extra: BTreeMap::new(),
        }
    }
}
fn unquote(v: &str) -> &str {
    v.strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .unwrap_or(v)
}
fn parse_list(v: &str) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(v).unwrap_or_default()
}
impl Settings {
    pub fn load(path: &Path) -> Self {
        let Ok(text) = fs::read_to_string(path) else {
            return Self::default();
        };
        Self::parse(&text)
    }
    pub fn parse(text: &str) -> Self {
        let mut s = Self::default();
        for line in text.lines() {
            let Some((k, v)) = line.split_once(':') else {
                continue;
            };
            let f = |d: f32| v.parse::<f32>().unwrap_or(d);
            let b = |d: bool| match v {
                "true" => true,
                "false" => false,
                _ => d,
            };
            let i = |d: i32| v.parse::<i32>().unwrap_or(d);
            match k {
                "version" => {}
                "fov" => {
                    s.fov = v
                        .parse::<f32>()
                        .map_or(70.0, |f| (f * 40.0 + 70.0).round().clamp(30.0, 110.0))
                }
                "renderDistance" => s.render_distance = i(12).clamp(2, 32),
                "simulationDistance" => s.simulation_distance = i(12).clamp(5, 32),
                "mouseSensitivity" => s.sensitivity = f(0.5).clamp(0.0, 1.0),
                "guiScale" => s.gui_scale = v.parse().unwrap_or(0),
                "bobView" => s.view_bobbing = b(true),
                "gamma" => s.gamma = f(0.5).clamp(0.0, 1.0),
                "maxFps" => s.max_fps = v.parse::<u32>().unwrap_or(260).clamp(10, 260),
                "enableVsync" => s.vsync = b(true),
                "fullscreen" => s.fullscreen = b(false),
                "ao" => s.smooth_lighting = b(true),
                "renderClouds" => s.clouds = unquote(v).to_owned(),
                "cloudRange" => s.cloud_range = i(128).clamp(2, 128),
                "particles" => s.particles = v.parse::<u8>().unwrap_or(0).min(2),
                "entityDistanceScaling" => s.entity_distance = f(1.0).clamp(0.5, 5.0),
                "entityShadows" => s.entity_shadows = b(true),
                "biomeBlendRadius" => s.biome_blend = i(2).clamp(0, 7),
                "graphicsPreset" => s.graphics_preset = unquote(v).to_owned(),
                "mipmapLevels" => s.mipmap_levels = i(4).clamp(0, 4),
                "weatherRadius" => s.weather_radius = i(10).clamp(3, 10),
                "chunkSectionFadeInTime" => s.chunk_fade = f(0.75).clamp(0.0, 2.0),
                "vignette" => s.vignette = b(true),
                "showAutosaveIndicator" => s.autosave_indicator = b(true),
                "cutoutLeaves" => s.cutout_leaves = b(true),
                "improvedTransparency" => s.improved_transparency = b(false),
                "attackIndicator" => s.attack_indicator = v.parse::<u8>().unwrap_or(1).min(2),
                "inactivityFpsLimit" => s.inactivity_fps_limit = unquote(v).to_owned(),
                "prioritizeChunkUpdates" => {
                    s.prioritize_chunk_updates = v.parse::<u8>().unwrap_or(0).min(2)
                }
                "menuBackgroundBlurriness" => s.menu_blur = i(5).clamp(0, 10),
                "invertYMouse" => s.invert_y = b(false),
                "mouseWheelSensitivity" => s.mouse_wheel_sensitivity = f(1.0).clamp(0.01, 10.0),
                "discrete_mouse_scroll" => s.discrete_scroll = b(false),
                "toggleCrouch" => s.toggle_crouch = b(false),
                "toggleSprint" => s.toggle_sprint = b(false),
                "autoJump" => s.auto_jump = b(false),
                "lang" => s.language = v.to_owned(),
                "forceUnicodeFont" => s.force_unicode = b(false),
                "japaneseGlyphVariants" => s.japanese_glyphs = b(false),
                "chatVisibility" => s.chat_visibility = v.parse::<u8>().unwrap_or(0).min(2),
                "chatColors" => s.chat_colors = b(true),
                "chatLinks" => s.chat_links = b(true),
                "chatOpacity" => s.chat_opacity = f(1.0).clamp(0.0, 1.0),
                "textBackgroundOpacity" => s.text_background_opacity = f(0.5).clamp(0.0, 1.0),
                "chatScale" => s.chat_scale = f(1.0).clamp(0.0, 1.0),
                "chatWidth" => s.chat_width = f(1.0).clamp(0.0, 1.0),
                "chatHeightFocused" => s.chat_height_focused = f(1.0).clamp(0.0, 1.0),
                "chatHeightUnfocused" => s.chat_height_unfocused = f(0.4375).clamp(0.0, 1.0),
                "chatLineSpacing" => s.chat_line_spacing = f(0.0).clamp(0.0, 1.0),
                "autoSuggestions" => s.auto_suggestions = b(true),
                "reducedDebugInfo" => s.reduced_debug_info = b(false),
                "highContrast" => s.high_contrast = b(false),
                "showSubtitles" => s.show_subtitles = b(false),
                "notificationDisplayTime" => {
                    s.notification_display_time = f(1.0).clamp(0.5, 10.0);
                }
                "mainHand" => s.main_hand_left = unquote(v) == "left",
                "resourcePacks" => s.resource_packs = parse_list(v),
                _ if k.starts_with("key_") => {
                    if !s.keys.set_saved(&k[4..], v) {
                        s.extra.insert(k.to_owned(), v.to_owned());
                    }
                }
                _ if k.starts_with("soundCategory_") => {
                    let name = &k["soundCategory_".len()..];
                    match SOUND_SOURCES.iter().position(|n| *n == name) {
                        Some(0) => s.master_volume = f(1.0).clamp(0.0, 1.0),
                        Some(i) => s.volumes[i] = f(1.0).clamp(0.0, 1.0),
                        None => {
                            s.extra.insert(k.to_owned(), v.to_owned());
                        }
                    }
                }
                _ => {
                    s.extra.insert(k.to_owned(), v.to_owned());
                }
            }
        }
        s
    }
    /// The `options.txt` text (Options.save: `version` first, then key:value).
    pub fn serialize(&self) -> String {
        let mut lines: Vec<(String, String)> = vec![
            ("ao".into(), self.smooth_lighting.to_string()),
            ("biomeBlendRadius".into(), self.biome_blend.to_string()),
            ("chunkSectionFadeInTime".into(), self.chunk_fade.to_string()),
            ("cutoutLeaves".into(), self.cutout_leaves.to_string()),
            ("enableVsync".into(), self.vsync.to_string()),
            (
                "entityDistanceScaling".into(),
                self.entity_distance.to_string(),
            ),
            ("entityShadows".into(), self.entity_shadows.to_string()),
            ("forceUnicodeFont".into(), self.force_unicode.to_string()),
            (
                "japaneseGlyphVariants".into(),
                self.japanese_glyphs.to_string(),
            ),
            ("fov".into(), ((self.fov - 70.0) / 40.0).to_string()),
            (
                "graphicsPreset".into(),
                format!("\"{}\"", self.graphics_preset),
            ),
            (
                "prioritizeChunkUpdates".into(),
                self.prioritize_chunk_updates.to_string(),
            ),
            ("fullscreen".into(), self.fullscreen.to_string()),
            ("gamma".into(), self.gamma.to_string()),
            ("guiScale".into(), self.gui_scale.to_string()),
            ("maxFps".into(), self.max_fps.to_string()),
            (
                "improvedTransparency".into(),
                self.improved_transparency.to_string(),
            ),
            (
                "inactivityFpsLimit".into(),
                format!("\"{}\"", self.inactivity_fps_limit),
            ),
            ("mipmapLevels".into(), self.mipmap_levels.to_string()),
            ("particles".into(), self.particles.to_string()),
            (
                "reducedDebugInfo".into(),
                self.reduced_debug_info.to_string(),
            ),
            ("renderClouds".into(), format!("\"{}\"", self.clouds)),
            ("cloudRange".into(), self.cloud_range.to_string()),
            ("renderDistance".into(), self.render_distance.to_string()),
            (
                "simulationDistance".into(),
                self.simulation_distance.to_string(),
            ),
            ("vignette".into(), self.vignette.to_string()),
            ("weatherRadius".into(), self.weather_radius.to_string()),
            ("autoJump".into(), self.auto_jump.to_string()),
            ("autoSuggestions".into(), self.auto_suggestions.to_string()),
            ("chatColors".into(), self.chat_colors.to_string()),
            ("chatLinks".into(), self.chat_links.to_string()),
            (
                "discrete_mouse_scroll".into(),
                self.discrete_scroll.to_string(),
            ),
            ("invertYMouse".into(), self.invert_y.to_string()),
            ("bobView".into(), self.view_bobbing.to_string()),
            ("toggleCrouch".into(), self.toggle_crouch.to_string()),
            ("toggleSprint".into(), self.toggle_sprint.to_string()),
            ("mouseSensitivity".into(), self.sensitivity.to_string()),
            ("highContrast".into(), self.high_contrast.to_string()),
            ("showSubtitles".into(), self.show_subtitles.to_string()),
            (
                "notificationDisplayTime".into(),
                self.notification_display_time.to_string(),
            ),
            (
                "resourcePacks".into(),
                serde_json::to_string(&self.resource_packs).unwrap_or_else(|_| "[]".into()),
            ),
            ("lang".into(), self.language.clone()),
            ("chatVisibility".into(), self.chat_visibility.to_string()),
            ("chatOpacity".into(), self.chat_opacity.to_string()),
            ("chatLineSpacing".into(), self.chat_line_spacing.to_string()),
            (
                "textBackgroundOpacity".into(),
                self.text_background_opacity.to_string(),
            ),
            (
                "chatHeightFocused".into(),
                self.chat_height_focused.to_string(),
            ),
            (
                "chatHeightUnfocused".into(),
                self.chat_height_unfocused.to_string(),
            ),
            ("chatScale".into(), self.chat_scale.to_string()),
            ("chatWidth".into(), self.chat_width.to_string()),
            (
                "mainHand".into(),
                if self.main_hand_left {
                    "\"left\"".into()
                } else {
                    "\"right\"".into()
                },
            ),
            ("attackIndicator".into(), self.attack_indicator.to_string()),
            (
                "mouseWheelSensitivity".into(),
                self.mouse_wheel_sensitivity.to_string(),
            ),
            (
                "showAutosaveIndicator".into(),
                self.autosave_indicator.to_string(),
            ),
            (
                "menuBackgroundBlurriness".into(),
                self.menu_blur.to_string(),
            ),
        ];
        for (name, key) in self.keys.saved() {
            lines.push((format!("key_{name}"), key));
        }
        for (i, source) in SOUND_SOURCES.iter().enumerate() {
            let v = if i == 0 {
                self.master_volume
            } else {
                self.volumes[i]
            };
            lines.push((format!("soundCategory_{source}"), v.to_string()));
        }
        let known: std::collections::BTreeSet<String> =
            lines.iter().map(|(k, _)| k.clone()).collect();
        for (k, v) in &self.extra {
            if !known.contains(k) {
                lines.push((k.clone(), v.clone()));
            }
        }
        let mut out = format!("version:{}\n", horizon_world::DATA_VERSION);
        for (k, v) in lines {
            out.push_str(&format!("{k}:{v}\n"));
        }
        out
    }
    pub fn save(&self, path: &Path) {
        let _ = fs::write(path, self.serialize());
    }
    /// Volume of a sound source (master excluded).
    pub fn volume(&self, source: &str) -> f32 {
        SOUND_SOURCES
            .iter()
            .position(|n| *n == source)
            .map_or(1.0, |i| {
                if i == 0 {
                    self.master_volume
                } else {
                    self.volumes[i]
                }
            })
    }
    pub fn chat_options(&self) -> crate::chat::ChatOptions {
        crate::chat::ChatOptions {
            opacity: self.chat_opacity,
            background_opacity: self.text_background_opacity,
            scale: self.chat_scale,
            width: self.chat_width,
            height_focused: self.chat_height_focused,
            height_unfocused: self.chat_height_unfocused,
            line_spacing: self.chat_line_spacing,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn round_trips_vanilla_options_txt() {
        let text = "version:4000\nfov:0.5\nrenderDistance:16\nmaxFps:260\nrenderClouds:\"fast\"\nkey_key.jump:key.keyboard.j\nsoundCategory_music:0.25\nresourcePacks:[\"vanilla\",\"file/x.zip\"]\nlang:fr_fr\nsomeFutureOption:42\n";
        let s = Settings::parse(text);
        assert_eq!(s.fov, 90.0);
        assert_eq!(s.render_distance, 16);
        assert_eq!(s.max_fps, 260);
        assert_eq!(s.clouds, "fast");
        assert_eq!(s.keys.get("key.jump"), "key.keyboard.j");
        assert_eq!(s.volume("music"), 0.25);
        assert_eq!(s.resource_packs, ["vanilla", "file/x.zip"]);
        assert_eq!(s.language, "fr_fr");
        let out = s.serialize();
        assert!(out.starts_with("version:"));
        assert!(out.contains("\nkey_key.jump:key.keyboard.j\n"));
        assert!(out.contains("\nsomeFutureOption:42\n"));
        assert!(out.contains("\nrenderClouds:\"fast\"\n"));
        let again = Settings::parse(&out);
        assert_eq!(again.fov, 90.0);
        assert_eq!(again.keys.get("key.jump"), "key.keyboard.j");
    }
    #[test]
    fn subtitle_options() {
        let s = Settings::parse("showSubtitles:true\nnotificationDisplayTime:2.5\n");
        assert!(s.show_subtitles);
        assert_eq!(s.notification_display_time, 2.5);
        let again = Settings::parse(&s.serialize());
        assert!(again.show_subtitles);
        assert_eq!(again.notification_display_time, 2.5);
        let d = Settings::parse("");
        assert!(!d.show_subtitles);
        assert_eq!(d.notification_display_time, 1.0);
        // The multiplier stays within the slider's range.
        assert_eq!(
            Settings::parse("notificationDisplayTime:50\n").notification_display_time,
            10.0
        );
    }
}
