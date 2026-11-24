//! Horizon's optional extras (minimap, lighting, inventory tools): their
//! settings, kept in `horizon-extras.txt` next to `options.txt` so that file
//! stays exactly vanilla's, and the logic behind them. Every extra is off by
//! default; see `docs/extras.md`. Horizon's own switches outside the Extras page
//! (No AI, Agent Control) are kept here too.
use std::{collections::VecDeque, fs, path::Path};

pub const FILE: &str = "horizon-extras.txt";
/// The name the options screen uses while the sort key waits for a key.
pub const SORT_KEY: &str = "horizon.sort";
/// The 3D skin layers line, here and in the `options.txt` that held it first.
const SKIN_LAYERS_KEY: &str = "threeDSkinLayers";
/// The emote wheel's key, here and in the `key_key.emote` line #10092 wrote to `options.txt`.
const EMOTE_KEY: &str = "emoteKey";
/// The fullbright toggle's key (`key.fullbright`), which vanilla has no line for at all.
const FULLBRIGHT_KEY: &str = "fullbrightKey";
const OLD_EMOTE_LINE: &str = "key_key.emote";

/// The minimap's sizes in GUI pixels, and its zooms in blocks per pixel.
pub const MINIMAP_SIZES: [(u8, &str); 3] = [(64, "Small"), (96, "Medium"), (128, "Large")];
pub const MINIMAP_ZOOMS: [f32; 4] = [0.5, 1.0, 2.0, 4.0];

/// A screen corner for an overlay.
pub use mod_api::client::extras::Corner;

/// The extras official mods took over, by the mod's `horizon-mod.toml` id, with the keys of
/// this file that held the extra's settings. A port reads them once to carry the player's
/// choices over (`Extras::moved_settings`, the mod API's `official_extra_settings`).
/// `lodZoomDetail` stays with the renderer, not the Zoom mod.
pub const MOVED_KEYS: &[(&str, &[&str])] = &[
    ("fps_overlay", &["fps", "fpsCorner", "fpsLows"]),
    (
        "keystrokes",
        &["keystrokes", "keystrokesCorner", "keystrokesCps"],
    ),
    (
        "minimap",
        &[
            "minimap",
            "minimapCorner",
            "minimapSize",
            "minimapShape",
            "minimapZoom",
            "minimapRotate",
        ],
    ),
    (
        "zoom",
        &[
            "zoom",
            "zoomKey",
            "zoomLevel",
            "zoomSmooth",
            "zoomSensitivity",
        ],
    ),
];

#[derive(Clone, Debug, PartialEq)]
pub struct Extras {
    /// The Select Resource Packs screen's Add button, which browses
    /// Modrinth. Off by default: it is Horizon's own and the screen is
    /// otherwise vanilla's (tools/test_pack_screen_parity.py).
    pub pack_browser: bool,
    /// Make /pregen available in single-player and hosted LAN worlds.
    pub pregen: bool,
    pub fullbright: bool,
    /// Held, dropped and burning light sources light their surroundings (`dynamic_light`).
    pub dynamic_lights: bool,
    pub zoom: bool,
    /// A vanilla key name (`key.keyboard.z`).
    pub zoom_key: String,
    /// The zoom the key starts at.
    pub zoom_level: u8,
    /// Ease through vanilla's FOV easing (as the spyglass does).
    pub zoom_smooth: bool,
    /// Divide mouse turning by the zoom.
    pub zoom_sensitivity: bool,
    /// Keep a scoped look's sideways screen speed near the poles: the yaw of a spyglass turn
    /// is scaled by `1/cos(pitch)`, capped at 8× (`spyglass_polar_look::yaw_scale`). Off by
    /// default: vanilla turns both axes at the same rate, and the maintainer decides whether
    /// this should ever be the default.
    pub spyglass_polar_look: bool,
    /// Far terrain a zoom magnifies is planned finer, so a spyglass does not
    /// show the coarse cells it was given for an unzoomed screen
    /// (`lod::Aim`). Costs far tiles while the zoom is held and gives them
    /// back after, and does nothing at all unless far terrain is on
    /// (`lodDistance`) and the view is actually zoomed. Off by default until
    /// a measurement shows the extra tiles land while the zoom is still held.
    pub lod_zoom_detail: bool,
    /// The FPS counter and keystrokes extras are official mods now: their
    /// old settings stay here, kept as they were.
    pub fps: bool,
    pub fps_corner: Corner,
    /// Also show the minimum and 1% low.
    pub fps_lows: bool,
    pub keystrokes: bool,
    pub keystrokes_corner: Corner,
    /// Clicks per second under the mouse buttons.
    pub keystrokes_cps: bool,
    /// The official mods once installed for the extras they replace, by mod
    /// id (`movedToMods`).
    pub moved: Vec<String>,
    pub minimap: bool,
    pub minimap_corner: Corner,
    /// Side in GUI pixels (`MINIMAP_SIZES`).
    pub minimap_size: u8,
    pub minimap_round: bool,
    /// Blocks per pixel (`MINIMAP_ZOOMS`).
    pub minimap_zoom: f32,
    /// Turn with the player (forward up) instead of north up.
    pub minimap_rotate: bool,
    /// Retry lost multiplayer connections (`reconnect`).
    pub auto_reconnect: bool,
    /// Positional sounds leave a few static sound channels free for UI sounds
    /// and others played to the listener (`sound_engine::HEARD_RESERVE`).
    pub ui_sound_reserve: bool,
    /// NeoForge's `skipModdedSplashesIfVanillaModified`, on by default: a
    /// pack's own `minecraft` splashes replace every namespace's
    /// (`horizon_client::splashes::pool`). No Extras page entry.
    pub skip_modded_splashes_if_vanilla_modified: bool,
    /// No particles from breaking or mining a block (other particles stay).
    pub hide_block_particles: bool,
    /// Mouse Tweaks-style drags in the inventory and storage containers.
    pub mouse_tweaks: bool,
    /// With them, the wheel over a slot moves its items one at a time.
    pub mouse_tweaks_wheel: bool,
    /// Sort buttons on the inventory and chest-like containers.
    pub sort: bool,
    /// A vanilla key name: sorts the part under the cursor.
    pub sort_key: String,
    /// The emote wheel's key (`key.emote`), as the game last saved it. The live binding is
    /// `Settings::keys`, bound in Controls with the vanilla ones; it is only kept in this file
    /// so that `options.txt` stays vanilla's (`Settings::load` hands it over, `save` takes it back).
    pub emote_key: String,
    /// Fullbright's key (`key.fullbright`), kept here like `emote_key`: unbound by default, so
    /// the switch in the Video settings is the only way to it until a player picks a key.
    pub fullbright_key: String,
    /// A new chat line slides in from the left while it fades in (`Chat::layout`).
    pub chat_animation: bool,
    /// The indicator bar at the left of a chat line (on by default, as vanilla).
    pub chat_indicator: bool,
    /// No AI: every AI feature hidden and off (`horizon_core::no_ai`; the launcher writes it too).
    pub no_ai: bool,
    /// Agents may control the game the player starts: it opens its control socket for
    /// horizon-ctl and MCP clients (off by default; an agent's own launch always opens it).
    pub agent_control: bool,
    /// F3+R and F3+E start/stop a repro recording and write its bundle (off by
    /// default: a recording copies the world).
    pub repro_keys: bool,
    /// Another controller can join P1's world (`splitscreen`, off by default).
    pub splitscreen: bool,
    /// Two players in one window: 0 auto (side by side when wider than tall), 1 side by side,
    /// 2 top and bottom (`splitscreenTwo`).
    pub splitscreen_two: u8,
    /// Three players on a screen: 0 one wide on top, 1 one tall on the left, 2 side by side
    /// (`splitscreenThree`).
    pub splitscreen_three: u8,
    /// Splitscreen: each player's name in the corner of their view (`splitscreenNames`).
    pub splitscreen_names: bool,
    /// The wheel over a cycle button in the settings changes it (`CycleButton.mouseScrolled`,
    /// on by default as vanilla); off, the wheel only scrolls the list.
    pub wheel_cycles_settings: bool,
    /// The built-in music tracks the Music page turned off (sound ids): they
    /// leave every pool they play in (`musicOff`, empty by default: vanilla's
    /// random selection).
    pub music_off: Vec<String>,
    /// The Music page: the Music... button on Music & Sounds, playing a
    /// built-in track and leaving the off ones out (`musicPage`, off by
    /// default: vanilla plays exactly as before).
    pub music_page: bool,
    /// 3D skin layers (`threeDSkinLayers`, Options → Skin Customization): each
    /// painted texel of a player's outer layer drawn as a slab
    /// (`horizon_client::mobs::skin_layers`).
    pub three_d_skin_layers: bool,
    /// The PvP & HUD page's options (`pvp`).
    pub pvp: crate::pvp::Pvp,
}
impl Default for Extras {
    fn default() -> Self {
        Self {
            pack_browser: false,
            pregen: false,
            fullbright: false,
            dynamic_lights: false,
            zoom: false,
            zoom_key: "key.keyboard.z".into(),
            zoom_level: 4,
            zoom_smooth: true,
            zoom_sensitivity: true,
            spyglass_polar_look: false,
            lod_zoom_detail: false,
            fps: false,
            fps_corner: Corner::TopLeft,
            fps_lows: false,
            keystrokes: false,
            keystrokes_corner: Corner::TopLeft,
            keystrokes_cps: true,
            moved: Vec::new(),
            minimap: false,
            minimap_corner: Corner::TopRight,
            minimap_size: 96,
            minimap_round: false,
            minimap_zoom: 1.0,
            minimap_rotate: false,
            auto_reconnect: false,
            ui_sound_reserve: false,
            skip_modded_splashes_if_vanilla_modified: true,
            hide_block_particles: false,
            mouse_tweaks: false,
            mouse_tweaks_wheel: true,
            sort: false,
            sort_key: "key.keyboard.unknown".into(),
            emote_key: crate::keybinds::KeyBinds::default_of("key.emote").into(),
            fullbright_key: crate::keybinds::KeyBinds::default_of("key.fullbright").into(),
            chat_animation: false,
            chat_indicator: true,
            no_ai: false,
            agent_control: false,
            repro_keys: false,
            splitscreen: false,
            splitscreen_two: 0,
            splitscreen_three: 0,
            splitscreen_names: true,
            wheel_cycles_settings: false,
            music_off: Vec::new(),
            music_page: false,
            three_d_skin_layers: false,
            pvp: Default::default(),
        }
    }
}
/// The largest file `Extras::moved_settings_in` reads: far above any extras file the game
/// writes (a few KiB), so a huge or hostile file can't exhaust memory.
pub const MOVED_READ_MAX: u64 = 1 << 20;
/// `path` as text when it is a regular file (a link to one counts) of at most
/// `MOVED_READ_MAX` bytes in UTF-8. Checked before opening, so a device or pipe is never
/// opened, and again on the open file, in case it was swapped in between.
fn read_bounded(path: &Path) -> Option<String> {
    use std::io::Read;
    let small = |meta: fs::Metadata| meta.is_file() && meta.len() <= MOVED_READ_MAX;
    if !small(fs::metadata(path).ok()?) {
        return None;
    }
    let file = fs::File::open(path).ok()?;
    if !small(file.metadata().ok()?) {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(MOVED_READ_MAX + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > MOVED_READ_MAX {
        return None;
    }
    String::from_utf8(bytes).ok()
}
fn bool_text(v: bool) -> &'static str {
    if v { "true" } else { "false" }
}
impl Extras {
    /// The settings official mod `mod_id` took over (`MOVED_KEYS`), from this file's `text`:
    /// each of its keys that has a line there, with the value the extra took from it, as
    /// `serialize` writes it: the last line it can read wins, and a key with no readable line
    /// gives its default.
    /// Empty for any other mod.
    pub fn moved_settings(text: &str, mod_id: &str) -> Vec<(String, String)> {
        let Some(&(_, keys)) = MOVED_KEYS.iter().find(|(id, _)| *id == mod_id) else {
            return Vec::new();
        };
        let written = |key: &str| {
            text.lines()
                .any(|line| line.split_once(':').is_some_and(|(k, _)| k.trim() == key))
        };
        Self::parse(text)
            .serialize()
            .lines()
            .filter_map(|line| line.split_once(':'))
            .filter(|(key, _)| keys.contains(key) && written(key))
            .map(|(key, value)| (key.to_owned(), value.to_owned()))
            .collect()
    }
    /// `moved_settings` of this file in `dir`. The file is read only for a mod with a
    /// `MOVED_KEYS` row, and only when it is a regular file of at most `MOVED_READ_MAX` bytes
    /// in UTF-8; anything else gives nothing.
    pub fn moved_settings_in(dir: &Path, mod_id: &str) -> Vec<(String, String)> {
        Self::moved_settings_read(dir, mod_id, read_bounded)
    }
    fn moved_settings_read(
        dir: &Path,
        mod_id: &str,
        read: fn(&Path) -> Option<String>,
    ) -> Vec<(String, String)> {
        if !MOVED_KEYS.iter().any(|(id, _)| *id == mod_id) {
            return Vec::new();
        }
        read(&dir.join(FILE)).map_or_else(Vec::new, |text| Self::moved_settings(&text, mod_id))
    }
    pub fn load(dir: &Path) -> Self {
        fs::read_to_string(dir.join(FILE))
            .map(|t| Self::parse(&t))
            .unwrap_or_default()
    }
    pub fn parse(text: &str) -> Self {
        let mut e = Self::default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once(':') else {
                continue;
            };
            let value = value.trim();
            let flag = |d: bool| match value {
                "true" => true,
                "false" => false,
                _ => d,
            };
            match key.trim() {
                "packBrowser" => e.pack_browser = flag(e.pack_browser),
                "pregen" => e.pregen = flag(e.pregen),
                "fullbright" => e.fullbright = flag(e.fullbright),
                "dynamicLights" => e.dynamic_lights = flag(e.dynamic_lights),
                "zoom" => e.zoom = flag(e.zoom),
                "zoomKey" if value.starts_with("key.") => e.zoom_key = value.to_owned(),
                "zoomLevel" => {
                    if let Ok(v) = value.parse::<u8>() {
                        e.zoom_level = v.clamp(2, 10);
                    }
                }
                "zoomSmooth" => e.zoom_smooth = flag(e.zoom_smooth),
                "zoomSensitivity" => e.zoom_sensitivity = flag(e.zoom_sensitivity),
                "spyglassPolarLook" => e.spyglass_polar_look = flag(e.spyglass_polar_look),
                "lodZoomDetail" => e.lod_zoom_detail = flag(e.lod_zoom_detail),
                "fps" => e.fps = flag(e.fps),
                "fpsCorner" => e.fps_corner = Corner::parse(value, e.fps_corner),
                "fpsLows" => e.fps_lows = flag(e.fps_lows),
                "keystrokes" => e.keystrokes = flag(e.keystrokes),
                "keystrokesCorner" => {
                    e.keystrokes_corner = Corner::parse(value, e.keystrokes_corner)
                }
                "keystrokesCps" => e.keystrokes_cps = flag(e.keystrokes_cps),
                "movedToMods" => {
                    e.moved = value
                        .split(',')
                        .map(str::trim)
                        .filter(|id| !id.is_empty())
                        .map(str::to_owned)
                        .collect()
                }
                "minimap" => e.minimap = flag(e.minimap),
                "minimapCorner" => e.minimap_corner = Corner::parse(value, e.minimap_corner),
                "minimapSize" => {
                    if let Some(&(v, _)) = MINIMAP_SIZES.iter().find(|s| s.0.to_string() == value) {
                        e.minimap_size = v;
                    }
                }
                "minimapShape" => e.minimap_round = value == "round",
                "minimapZoom" => {
                    if let Some(&z) = MINIMAP_ZOOMS.iter().find(|z| z.to_string() == value) {
                        e.minimap_zoom = z;
                    }
                }
                "minimapRotate" => e.minimap_rotate = flag(e.minimap_rotate),
                "autoReconnect" => e.auto_reconnect = flag(e.auto_reconnect),
                "uiSoundReserve" => e.ui_sound_reserve = flag(e.ui_sound_reserve),
                "skipModdedSplashesIfVanillaModified" => {
                    e.skip_modded_splashes_if_vanilla_modified =
                        flag(e.skip_modded_splashes_if_vanilla_modified)
                }
                "hideBlockParticles" => e.hide_block_particles = flag(e.hide_block_particles),
                "mouseTweaks" => e.mouse_tweaks = flag(e.mouse_tweaks),
                "mouseTweaksWheel" => e.mouse_tweaks_wheel = flag(e.mouse_tweaks_wheel),
                "sort" => e.sort = flag(e.sort),
                "sortKey" if value.starts_with("key.") => e.sort_key = value.to_owned(),
                EMOTE_KEY if value.starts_with("key.") => e.emote_key = value.to_owned(),
                FULLBRIGHT_KEY if value.starts_with("key.") => e.fullbright_key = value.to_owned(),
                "chatAnimation" => e.chat_animation = flag(e.chat_animation),
                "chatIndicator" => e.chat_indicator = flag(e.chat_indicator),
                horizon_core::no_ai::KEY => e.no_ai = flag(e.no_ai),
                "agentControl" => e.agent_control = flag(e.agent_control),
                "reproKeys" => e.repro_keys = flag(e.repro_keys),
                "splitscreen" => e.splitscreen = flag(e.splitscreen),
                "splitscreenTwo" => e.splitscreen_two = value.parse().unwrap_or(0).min(2),
                "splitscreenThree" => e.splitscreen_three = value.parse().unwrap_or(0).min(2),
                "splitscreenNames" => e.splitscreen_names = flag(e.splitscreen_names),
                // A new name: `wheelCyclesSettings` was written as true by every
                // install while it was the default, so it says nothing of a choice.
                "wheelCyclesListSettings" => {
                    e.wheel_cycles_settings = flag(e.wheel_cycles_settings)
                }
                "musicOff" => {
                    e.music_off = value
                        .split(',')
                        .map(str::trim)
                        .filter(|id| !id.is_empty())
                        .map(str::to_owned)
                        .collect()
                }
                "musicPage" => e.music_page = flag(e.music_page),
                SKIN_LAYERS_KEY => e.three_d_skin_layers = flag(e.three_d_skin_layers),
                key => {
                    e.pvp.parse_line(key, value);
                }
            }
        }
        e
    }
    pub fn serialize(&self) -> String {
        let mut text = format!(
            "packBrowser:{}\npregen:{}\nfullbright:{}\ndynamicLights:{}\nzoom:{}\nzoomKey:{}\nzoomLevel:{}\nzoomSmooth:{}\nzoomSensitivity:{}\nlodZoomDetail:{}\nfps:{}\nfpsCorner:{}\nfpsLows:{}\nkeystrokes:{}\nkeystrokesCorner:{}\nkeystrokesCps:{}\nmovedToMods:{}\nminimap:{}\nminimapCorner:{}\nminimapSize:{}\nminimapShape:{}\nminimapZoom:{}\nminimapRotate:{}\nautoReconnect:{}\nuiSoundReserve:{}\nskipModdedSplashesIfVanillaModified:{}\nhideBlockParticles:{}\nmouseTweaks:{}\nmouseTweaksWheel:{}\nsort:{}\nsortKey:{}\nchatAnimation:{}\nchatIndicator:{}\n{}:{}\nagentControl:{}\nreproKeys:{}\nsplitscreen:{}\nsplitscreenTwo:{}\nsplitscreenThree:{}\nsplitscreenNames:{}\nwheelCyclesListSettings:{}\nmusicOff:{}\nmusicPage:{}\n{}:{}\nspyglassPolarLook:{}\n",
            bool_text(self.pack_browser),
            bool_text(self.pregen),
            bool_text(self.fullbright),
            bool_text(self.dynamic_lights),
            bool_text(self.zoom),
            self.zoom_key,
            self.zoom_level,
            bool_text(self.zoom_smooth),
            bool_text(self.zoom_sensitivity),
            bool_text(self.lod_zoom_detail),
            bool_text(self.fps),
            self.fps_corner.name(),
            bool_text(self.fps_lows),
            bool_text(self.keystrokes),
            self.keystrokes_corner.name(),
            bool_text(self.keystrokes_cps),
            self.moved.join(","),
            bool_text(self.minimap),
            self.minimap_corner.name(),
            self.minimap_size,
            if self.minimap_round {
                "round"
            } else {
                "square"
            },
            self.minimap_zoom,
            bool_text(self.minimap_rotate),
            bool_text(self.auto_reconnect),
            bool_text(self.ui_sound_reserve),
            bool_text(self.skip_modded_splashes_if_vanilla_modified),
            bool_text(self.hide_block_particles),
            bool_text(self.mouse_tweaks),
            bool_text(self.mouse_tweaks_wheel),
            bool_text(self.sort),
            self.sort_key,
            bool_text(self.chat_animation),
            bool_text(self.chat_indicator),
            horizon_core::no_ai::KEY,
            bool_text(self.no_ai),
            bool_text(self.agent_control),
            bool_text(self.repro_keys),
            bool_text(self.splitscreen),
            self.splitscreen_two,
            self.splitscreen_three,
            bool_text(self.splitscreen_names),
            bool_text(self.wheel_cycles_settings),
            self.music_off.join(","),
            bool_text(self.music_page),
            SKIN_LAYERS_KEY,
            bool_text(self.three_d_skin_layers),
            bool_text(self.spyglass_polar_look),
        );
        text.push_str(&format!("{EMOTE_KEY}:{}\n", self.emote_key));
        text.push_str(&format!("{FULLBRIGHT_KEY}:{}\n", self.fullbright_key));
        self.pvp.serialize(&mut text);
        text
    }
    /// Adopts the settings `options.txt` held before they moved here, while
    /// this file (`own`) has no line of its own: 3D skin layers and the emote key were
    /// `options.txt` lines. Vanilla's `Options.save` drops lines it doesn't
    /// know, so it is read there once, the first start after the update.
    pub fn read_old_options(&mut self, own: &str, options_txt: &str) {
        fn line<'a>(text: &'a str, name: &str) -> Option<&'a str> {
            text.lines().find_map(|l| {
                l.split_once(':')
                    .filter(|(key, _)| *key == name)
                    .map(|(_, value)| value)
            })
        }
        if line(own, SKIN_LAYERS_KEY).is_none()
            && let Some(value) = line(options_txt, SKIN_LAYERS_KEY)
        {
            self.three_d_skin_layers = value == "true";
        }
        // The emote key was a `key_key.emote` line of options.txt (#10092): a rebound key is kept.
        if line(own, EMOTE_KEY).is_none()
            && let Some(value) = line(options_txt, OLD_EMOTE_LINE).map(str::trim)
            && value.starts_with("key.")
            && (value == "key.keyboard.unknown"
                || crate::keybinds::key_code(value).is_some()
                || crate::keybinds::mouse_button(value).is_some())
        {
            self.emote_key = value.to_owned();
        }
    }
    /// Binds the extra key `name` (`SORT_KEY`); false for
    /// another name (a vanilla mapping).
    pub fn set_key(&mut self, name: &str, value: String) -> bool {
        match name {
            SORT_KEY => self.sort_key = value,
            crate::pvp::HUD_EDITOR_KEY => self.pvp.editor_key = value,
            _ => return false,
        }
        true
    }
    /// Written only once something differs from the defaults, so a vanilla
    /// setup gains no file. The settings stores of this file follow these lines
    /// (`horizon_hooks::settings`).
    pub fn save(&self, dir: &Path) {
        let path = dir.join(FILE);
        let mut text = self.serialize();
        let stores_default = horizon_hooks::settings::engine::append(FILE, &mut text);
        if *self == Self::default() && stores_default && !path.exists() {
            return;
        }
        let _ = fs::write(path, text);
    }
}

/// The field-of-view modifier target `Camera.tickFov` eases toward: vanilla's
/// (`getFieldOfViewModifier`) divided by a mod's zoom (`native_zoom`). A
/// zoom of 1 leaves it as is.
pub fn fov_target(vanilla: f32, zoom: f32) -> f32 {
    if zoom == 1.0 { vanilla } else { vanilla / zoom }
}
/// Frame times over the last few seconds: frames per second, the minimum
/// and the 1% low, recomputed twice a second.
pub struct FpsCounter {
    /// (end time in seconds, frame milliseconds), oldest first.
    frames: VecDeque<(f64, f32)>,
    shown: FpsReading,
    computed_at: f64,
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FpsReading {
    pub fps: u32,
    pub min: u32,
    pub low: u32,
}
/// Seconds the minimum and 1% low look back.
const LOW_WINDOW: f64 = 5.0;
impl Default for FpsCounter {
    fn default() -> Self {
        Self {
            frames: VecDeque::new(),
            shown: FpsReading::default(),
            computed_at: f64::NEG_INFINITY,
        }
    }
}
impl FpsCounter {
    /// A frame that ended at `now` (seconds) and took `ms`.
    pub fn record(&mut self, now: f64, ms: f32) {
        self.frames.push_back((now, ms));
        while self
            .frames
            .front()
            .is_some_and(|&(t, _)| now - t > LOW_WINDOW)
        {
            self.frames.pop_front();
        }
        if now - self.computed_at >= 0.5 {
            self.computed_at = now;
            self.shown = self.compute(now);
        }
    }
    fn compute(&self, now: f64) -> FpsReading {
        let fps = self.frames.iter().filter(|&&(t, _)| now - t < 1.0).count() as u32;
        let mut times: Vec<f32> = self.frames.iter().map(|f| f.1).collect();
        if times.is_empty() {
            return FpsReading::default();
        }
        times.sort_by(|a, b| b.total_cmp(a));
        let to_fps = |ms: f32| (1000.0 / ms.max(0.001)).round() as u32;
        // The 1% low: the frame rate of the 99th percentile frame time.
        let low = times[(times.len() - 1) / 100];
        FpsReading {
            fps,
            min: to_fps(times[0]),
            low: to_fps(low),
        }
    }
    pub fn reading(&self) -> FpsReading {
        self.shown
    }
    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

/// Clicks of one button in the last second.
#[derive(Default)]
pub struct Cps {
    clicks: VecDeque<f64>,
}
impl Cps {
    pub fn click(&mut self, now: f64) {
        self.clicks.push_back(now);
        self.forget(now);
    }
    fn forget(&mut self, now: f64) {
        while self.clicks.front().is_some_and(|&t| now - t >= 1.0) {
            self.clicks.pop_front();
        }
    }
    pub fn per_second(&mut self, now: f64) -> usize {
        self.forget(now);
        self.clicks.len()
    }
}
/// Two players sharing a window (`splitscreen_two`): halves this way.
pub const SPLIT_TWO: [&str; 3] = ["Auto", "Side by side", "Top and bottom"];
/// Three players sharing a window (`splitscreen_three`).
pub const SPLIT_THREE: [&str; 3] = ["One wide on top", "One tall on the left", "Side by side"];

/// The settings the Extras page shows, one per row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    /// A spyglass look keeps its sideways screen speed near the poles
    /// (`spyglass_polar_look::yaw_scale`).
    SpyglassPolarLook,
    Fullbright,
    DynamicLights,
    HideBlockParticles,
    AutoReconnect,
    UiSoundReserve,
    /// The Music... page on Music & Sounds (`Music page`).
    MusicPage,
    MouseTweaks,
    MouseTweaksWheel,
    Sort,
    SortKey,
    /// A new chat line slides in and fades in (`Chat::layout`).
    ChatAnimation,
    /// The coloured bar at the left of a chat line.
    ChatIndicator,
    /// Not on the Extras page: Horizon Settings shows No AI, its AI Agent page Agent Control.
    NoAi,
    AgentControl,
    /// On the AI Agent page, next to Agent Control: the F3 recording shortcuts.
    ReproKeys,
    /// On the Splitscreen settings page.
    Splitscreen,
    WheelCyclesSettings,
    /// The Select Resource Packs screen's Add button (Modrinth).
    PackBrowser,
    Pregen,
    /// On the PvP & HUD page (`pvp::rows`).
    Pvp(crate::pvp::Opt),
}
/// The Extras page: headers (language keys) and items.
pub fn rows() -> Vec<Result<Item, &'static str>> {
    use Item::*;
    // Fullbright, Dynamic lights and Hide block-break particles change the picture: they
    // are on the graphics screen's Quality page (`graphics_ui::rows`), not here. The Cosmetics
    // group comes last, from the cosmetics' own section (`pages::Place::Extras`) while they are
    // offered.
    vec![
        Err("horizon.extras.header.look"),
        Ok(SpyglassPolarLook),
        // The own name tag's row (horizon-name-tag, a section: `horizon_hooks::pages`).
        Err("horizon.extras.header.third_person"),
        Err("horizon.extras.header.inventory"),
        Ok(MouseTweaks),
        Ok(MouseTweaksWheel),
        Ok(Sort),
        Ok(SortKey),
        Err("horizon.extras.header.chat"),
        Ok(ChatAnimation),
        Ok(ChatIndicator),
        Err("horizon.extras.header.world"),
        Ok(Pregen),
        Err("horizon.extras.header.multiplayer"),
        Ok(AutoReconnect),
        Err("horizon.extras.header.sound"),
        Ok(UiSoundReserve),
        Ok(MusicPage),
        Err("horizon.extras.header.menus"),
        Ok(WheelCyclesSettings),
        Ok(PackBrowser),
    ]
}
impl Item {
    /// The language key of the caption (`horizon.*`, assets/horizon/lang).
    pub fn caption_key(self) -> &'static str {
        match self {
            Item::SpyglassPolarLook => "horizon.extras.spyglass_polar_look",
            Item::Fullbright => "horizon.extras.fullbright",
            Item::DynamicLights => "horizon.extras.dynamic_lights",
            Item::HideBlockParticles => "horizon.extras.hide_block_particles",
            Item::AutoReconnect => "horizon.extras.auto_reconnect",
            Item::UiSoundReserve => "horizon.extras.ui_sound_reserve",
            Item::MusicPage => "horizon.extras.music_page",
            Item::MouseTweaks => "horizon.extras.mouse_tweaks",
            Item::MouseTweaksWheel => "horizon.extras.mouse_tweaks_wheel",
            Item::Sort => "horizon.extras.sort",
            Item::SortKey => "horizon.extras.sort_key",
            Item::NoAi => "horizon.extras.no_ai",
            Item::AgentControl => "horizon.extras.agent_control",
            Item::ReproKeys => "horizon.extras.repro_keys",
            Item::Splitscreen => "horizon.extras.splitscreen",
            Item::ChatAnimation => "horizon.extras.chat_animation",
            Item::ChatIndicator => "horizon.extras.chat_indicator",
            Item::WheelCyclesSettings => "horizon.extras.wheel_cycles_settings",
            Item::PackBrowser => "horizon.extras.pack_browser",
            Item::Pregen => "horizon.extras.pregen",
            Item::Pvp(o) => o.caption_key(),
        }
    }
    /// The language key of the description shown as the row's tooltip.
    pub fn description_key(self) -> &'static str {
        match self {
            Item::SpyglassPolarLook => "horizon.extras.spyglass_polar_look.tooltip",
            Item::Fullbright => "horizon.extras.fullbright.tooltip",
            Item::DynamicLights => "horizon.extras.dynamic_lights.tooltip",
            Item::HideBlockParticles => "horizon.extras.hide_block_particles.tooltip",
            Item::AutoReconnect => "horizon.extras.auto_reconnect.tooltip",
            Item::UiSoundReserve => "horizon.extras.ui_sound_reserve.tooltip",
            Item::MusicPage => "horizon.extras.music_page.tooltip",
            Item::MouseTweaks => "horizon.extras.mouse_tweaks.tooltip",
            Item::MouseTweaksWheel => "horizon.extras.mouse_tweaks_wheel.tooltip",
            Item::Sort => "horizon.extras.sort.tooltip",
            Item::SortKey => "horizon.extras.sort_key.tooltip",
            Item::NoAi => "horizon.extras.no_ai.tooltip",
            Item::AgentControl => "horizon.extras.agent_control.tooltip",
            Item::ReproKeys => "horizon.extras.repro_keys.tooltip",
            Item::Splitscreen => "horizon.extras.splitscreen.tooltip",
            Item::ChatAnimation => "horizon.extras.chat_animation.tooltip",
            Item::ChatIndicator => "horizon.extras.chat_indicator.tooltip",
            Item::WheelCyclesSettings => "horizon.extras.wheel_cycles_list_settings.tooltip",
            Item::PackBrowser => "horizon.extras.pack_browser.tooltip",
            Item::Pregen => "horizon.extras.pregen.tooltip",
            Item::Pvp(o) => o.description_key(),
        }
    }
    /// The caption in the current language (English when the key is missing).
    pub fn caption_text(self) -> String {
        crate::lang::get_or(self.caption_key(), self.caption())
    }
    /// The description in the current language (English when the key is missing).
    pub fn description_text(self) -> String {
        crate::lang::get_or(self.description_key(), self.description())
    }
    /// The English caption: the fallback of [`Item::caption_text`], and the text of
    /// `horizon.extras.*` in en_us.json.
    pub fn caption(self) -> &'static str {
        match self {
            Item::SpyglassPolarLook => "Spyglass look at the poles",
            Item::Fullbright => "Fullbright",
            Item::DynamicLights => "Dynamic lights",
            Item::HideBlockParticles => "Hide block-break particles",
            Item::AutoReconnect => "Auto-reconnect",
            Item::UiSoundReserve => "Keep channels for UI sounds",
            Item::MusicPage => "Music page",
            Item::MouseTweaks => "Mouse Tweaks controls",
            Item::MouseTweaksWheel => "Scroll wheel moves items",
            Item::Sort => "Sort buttons",
            Item::SortKey => "Sort key",
            Item::ChatAnimation => "Smooth chat",
            Item::ChatIndicator => "Message indicator",
            Item::NoAi => "No AI",
            Item::AgentControl => "Agent Control",
            Item::ReproKeys => "Recording shortcuts",
            Item::Splitscreen => "Splitscreen",
            Item::WheelCyclesSettings => "Scroll wheel changes settings",
            Item::PackBrowser => "Resource pack browser",
            Item::Pregen => "Chunk pregeneration",
            Item::Pvp(o) => o.caption(),
        }
    }
    /// The switch this setting belongs to: it does nothing while that is off.
    pub fn parent(self) -> Option<Item> {
        use Item::*;
        Some(match self {
            UiSoundReserve => return None,
            MouseTweaksWheel => MouseTweaks,
            SortKey => Sort,
            ChatIndicator => ChatAnimation,
            Pvp(o) => return o.parent().map(Pvp),
            Fullbright | DynamicLights | HideBlockParticles | MouseTweaks | Sort
            | AutoReconnect | NoAi | AgentControl | ReproKeys | Splitscreen | ChatAnimation
            | WheelCyclesSettings | MusicPage | PackBrowser | Pregen | SpyglassPolarLook => {
                return None;
            }
        })
    }
    /// The English description: the fallback of [`Item::description_text`].
    pub fn description(self) -> &'static str {
        match self {
            Item::SpyglassPolarLook => {
                "Looking up or down with a spyglass, the horizontal look gets slower as cos(pitch): this speeds it up by 1/cos(pitch), up to 8× (from 82.8°), so the picture moves sideways about as far as at the horizon. The turn also rolls the picture faster at those angles. Off, the look is vanilla's."
            }
            Item::Pregen => {
                "Enables /pregen in your single-player and LAN worlds. With cheats enabled, use /pregen start to generate an area before exploring it. Does not enable the command on servers you join."
            }
            Item::Fullbright => "Show blocks and entities at full light, even in dark areas.",
            Item::DynamicLights => {
                "Torches and other light sources held, dropped or on fire light the blocks around them."
            }
            Item::HideBlockParticles => {
                "Hides the debris particles that burst from a broken block and crumble off the one you mine. Other particles stay."
            }
            Item::AutoReconnect => {
                "Rejoins a server after a lost connection or a restart, never after a kick or ban."
            }
            Item::UiSoundReserve => {
                "Sounds in the world leave 16 of the 247 sound channels free, so a flood of them (a big explosion) can't silence menu clicks."
            }
            Item::MusicPage => {
                "Music & Sounds gets a Music... page: play any built-in track now, or turn one off so the game stops picking it. Nothing changes until you turn a track off."
            }
            Item::MouseTweaks => {
                "Right-drag places one per slot, left-drag gathers the held item, Shift + left-drag moves every slot."
            }
            Item::MouseTweaksWheel => {
                "Down sends one item of the slot to the other side, up brings one back."
            }
            Item::Sort => {
                "Sorts the inventory, chests, barrels, ender chests and shulker boxes in creative tab order, merging stacks."
            }
            Item::SortKey => {
                "Sorts the part under the cursor. Click, then press a key; Escape unbinds it."
            }
            Item::ChatAnimation => {
                "A new chat line slides in from the left while it fades in, instead of appearing whole."
            }
            Item::ChatIndicator => "The coloured bar at the left of a chat line.",
            Item::NoAi => {
                "Hides and turns off every AI feature: agent control, horizon-ctl and the MCP servers. The launcher's switch too."
            }
            Item::Splitscreen => {
                "In a world, another controller can join as a new player: press A when it asks, then pick a profile and where to play."
            }
            Item::AgentControl => {
                "Your own AI agent can play this game through horizon-ctl or an MCP client, on this computer only."
            }
            Item::ReproKeys => {
                "F3+R starts or stops a repro recording and F3+E writes its bundle. Off by default: a recording copies the world."
            }
            Item::Pvp(o) => o.description(),
            Item::WheelCyclesSettings => {
                "The scroll wheel over a button in the settings changes its value, as in vanilla. Off, the wheel only scrolls the list."
            }
            Item::PackBrowser => {
                "Select Resource Packs gets an Add button that searches Modrinth and installs a pack into your packs folder. Off, the game never contacts Modrinth."
            }
        }
    }
    /// The frame-time cost the main switch of an extra shows.
    pub fn cost(self) -> Option<crate::graphics_style::Cost> {
        use crate::graphics_style::Cost;
        match self {
            Item::Fullbright => Some(Cost::None),
            Item::UiSoundReserve => Some(Cost::None),
            Item::MusicPage => Some(Cost::None),
            Item::DynamicLights => Some(Cost::Low),
            Item::ChatAnimation => Some(Cost::None),
            Item::HideBlockParticles
            | Item::AutoReconnect
            | Item::MouseTweaks
            | Item::Sort
            | Item::Splitscreen
            | Item::WheelCyclesSettings
            | Item::SpyglassPolarLook
            | Item::PackBrowser => Some(Cost::None),
            _ => None,
        }
    }
    /// On/off items and their value.
    pub fn switch(self, e: &Extras) -> Option<bool> {
        Some(match self {
            Item::SpyglassPolarLook => e.spyglass_polar_look,
            Item::Fullbright => e.fullbright,
            Item::DynamicLights => e.dynamic_lights,
            Item::HideBlockParticles => e.hide_block_particles,
            Item::AutoReconnect => e.auto_reconnect,
            Item::UiSoundReserve => e.ui_sound_reserve,
            Item::MusicPage => e.music_page,
            Item::MouseTweaks => e.mouse_tweaks,
            Item::MouseTweaksWheel => e.mouse_tweaks_wheel,
            Item::Sort => e.sort,
            Item::ChatAnimation => e.chat_animation,
            Item::ChatIndicator => e.chat_indicator,
            Item::NoAi => e.no_ai,
            Item::AgentControl => e.agent_control,
            Item::ReproKeys => e.repro_keys,
            Item::Splitscreen => e.splitscreen,
            Item::WheelCyclesSettings => e.wheel_cycles_settings,
            Item::PackBrowser => e.pack_browser,
            Item::Pregen => e.pregen,
            Item::Pvp(o) => return o.switch(&e.pvp),
            _ => return None,
        })
    }
    /// The value of the other items, as their button shows it.
    pub fn value(self, e: &Extras) -> String {
        match self {
            Item::SortKey => crate::keybinds::display_name(&e.sort_key).plain(),
            Item::Pvp(o) if o.switch(&e.pvp).is_none() => o.value(&e.pvp).unwrap_or_default(),
            other => match other.switch(e) {
                Some(true) => crate::lang::get_or("On", "On"),
                _ => crate::lang::get_or("Off", "Off"),
            },
        }
    }
    /// A click (`back` for the right button: a PvP & HUD choice steps back; a switch flips
    /// either way). The sort key binds instead.
    pub fn cycle(self, e: &mut Extras, back: bool) {
        match self {
            Item::SpyglassPolarLook => e.spyglass_polar_look = !e.spyglass_polar_look,
            Item::Fullbright => e.fullbright = !e.fullbright,
            Item::DynamicLights => e.dynamic_lights = !e.dynamic_lights,
            Item::HideBlockParticles => e.hide_block_particles = !e.hide_block_particles,
            Item::AutoReconnect => e.auto_reconnect = !e.auto_reconnect,
            Item::UiSoundReserve => e.ui_sound_reserve = !e.ui_sound_reserve,
            Item::MusicPage => e.music_page = !e.music_page,
            Item::MouseTweaks => e.mouse_tweaks = !e.mouse_tweaks,
            Item::MouseTweaksWheel => e.mouse_tweaks_wheel = !e.mouse_tweaks_wheel,
            Item::Sort => e.sort = !e.sort,
            Item::ChatAnimation => e.chat_animation = !e.chat_animation,
            Item::ChatIndicator => e.chat_indicator = !e.chat_indicator,
            Item::NoAi => e.no_ai = !e.no_ai,
            Item::AgentControl => e.agent_control = !e.agent_control,
            Item::ReproKeys => e.repro_keys = !e.repro_keys,
            Item::Splitscreen => e.splitscreen = !e.splitscreen,
            Item::WheelCyclesSettings => e.wheel_cycles_settings = !e.wheel_cycles_settings,
            Item::PackBrowser => e.pack_browser = !e.pack_browser,
            Item::Pregen => e.pregen = !e.pregen,
            Item::Pvp(o) => o.cycle(&mut e.pvp, back),
            Item::SortKey => {}
        }
    }
    /// The key name the options screen waits on for a key item.
    pub fn key_name(self) -> Option<&'static str> {
        match self {
            Item::SortKey => Some(SORT_KEY),
            Item::Pvp(crate::pvp::Opt::HudEditorKey) => Some(crate::pvp::HUD_EDITOR_KEY),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An official mod gets its own extra's written keys, as the extra read them; any other
    /// mod, and a key the file lacks, gets nothing.
    #[test]
    fn moved_settings_are_the_calling_mods_written_keys() {
        let owned = |pairs: &[(&str, &str)]| -> Vec<(String, String)> {
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        };
        let text = "fps:true\nfpsCorner:nowhere\nkeystrokes:true\nzoomLevel:7\n";
        assert_eq!(
            Extras::moved_settings(text, "fps_overlay"),
            owned(&[("fps", "true"), ("fpsCorner", "top_left")])
        );
        let twice = "fpsCorner:top_right\nfpsCorner:bottom_right\n";
        assert_eq!(
            Extras::moved_settings(twice, "fps_overlay"),
            owned(&[("fpsCorner", "bottom_right")])
        );
        assert_eq!(
            Extras::moved_settings(text, "zoom"),
            owned(&[("zoomLevel", "7")])
        );
        assert!(Extras::moved_settings(text, "minimap").is_empty());
        assert!(Extras::moved_settings(text, "coords_hud").is_empty());
        assert!(Extras::moved_settings("", "fps_overlay").is_empty());
    }
    /// The file is read only for an official mod, and only as a small regular file.
    #[test]
    fn moved_settings_in_reads_only_a_small_file_for_an_official_mod() {
        let dir = tempfile::tempdir().unwrap();
        let unread = Extras::moved_settings_read(dir.path(), "coords_hud", |_| {
            panic!("read for a mod without moved keys")
        });
        assert!(unread.is_empty());
        let file = dir.path().join(FILE);
        fs::write(&file, "fpsLows:true\n").unwrap();
        assert_eq!(
            Extras::moved_settings_in(dir.path(), "fps_overlay"),
            [("fpsLows".to_owned(), "true".to_owned())]
        );
        let mut huge = "fpsLows:true\n".to_owned();
        huge.push_str(&"#".repeat(MOVED_READ_MAX as usize));
        fs::write(&file, huge).unwrap();
        assert!(Extras::moved_settings_in(dir.path(), "fps_overlay").is_empty());
        fs::remove_file(&file).unwrap();
        assert!(Extras::moved_settings_in(dir.path(), "fps_overlay").is_empty());
        fs::create_dir(&file).unwrap();
        assert!(Extras::moved_settings_in(dir.path(), "fps_overlay").is_empty());
    }
    #[test]
    fn a_saved_wheel_setting_from_before_the_default_changed_is_dropped() {
        assert!(!Extras::parse("wheelCyclesSettings:true\n").wheel_cycles_settings);
        let on = Extras {
            wheel_cycles_settings: true,
            ..Extras::default()
        };
        assert!(Extras::parse(&on.serialize()).wheel_cycles_settings);
    }

    #[test]
    fn everything_is_off_by_default_and_round_trips() {
        let d = Extras::default();
        assert!(!d.fullbright && !d.dynamic_lights);
        assert!(!d.zoom && !d.fps && !d.keystrokes && !d.minimap && !d.auto_reconnect);
        assert!(!d.mouse_tweaks && !d.sort && !d.ui_sound_reserve);
        assert!(!d.hide_block_particles && !d.pregen);
        assert!(!d.music_page && d.music_off.is_empty());
        assert!(!d.wheel_cycles_settings);
        assert_eq!(Extras::parse(""), d);
        assert_eq!(Extras::parse(&d.serialize()), d);
        let mut e = d.clone();
        Item::Fullbright.cycle(&mut e, false);
        assert_eq!(Item::Fullbright.switch(&e), Some(true));
        Item::DynamicLights.cycle(&mut e, false);
        assert_eq!(Item::DynamicLights.switch(&e), Some(true));
        assert_eq!(Extras::parse(&e.serialize()), e);
        e.zoom = true;
        e.zoom_key = "key.keyboard.c".into();
        e.zoom_level = 6;
        e.fps_corner = Corner::BottomRight;
        e.keystrokes = true;
        e.keystrokes_cps = false;
        e.moved = vec!["fps_overlay".into(), "keystrokes".into()];
        e.minimap = true;
        e.minimap_size = 128;
        e.minimap_round = true;
        e.minimap_zoom = 0.5;
        e.minimap_rotate = true;
        e.minimap_corner = Corner::BottomLeft;
        e.auto_reconnect = true;
        Item::Pregen.cycle(&mut e, false);
        assert_eq!(Item::Pregen.switch(&e), Some(true));
        assert_eq!(Item::HideBlockParticles.switch(&e), Some(false));
        Item::HideBlockParticles.cycle(&mut e, false);
        assert!(e.hide_block_particles);
        assert_eq!(Item::HideBlockParticles.switch(&e), Some(true));
        let only = Extras::parse("hideBlockParticles:true\n");
        assert!(only.hide_block_particles && !only.pregen);
        e.mouse_tweaks = true;
        e.mouse_tweaks_wheel = false;
        e.sort = true;
        e.sort_key = "key.keyboard.r".into();
        e.chat_animation = true;
        e.chat_indicator = false;
        assert!(!e.splitscreen);
        Item::Splitscreen.cycle(&mut e, false);
        assert_eq!(Item::Splitscreen.switch(&e), Some(true));
        Item::WheelCyclesSettings.cycle(&mut e, false);
        assert_eq!(Item::WheelCyclesSettings.switch(&e), Some(true));
        e.music_off = vec![
            "minecraft:music/game/calm1".into(),
            "minecraft:music.menu".into(),
        ];
        assert!(!e.music_page);
        Item::MusicPage.cycle(&mut e, false);
        assert_eq!(Item::MusicPage.switch(&e), Some(true));
        assert_eq!(Extras::parse(&e.serialize()), e);
    }

    #[test]
    fn bad_lines_keep_defaults() {
        let e = Extras::parse("zoom:maybe\nzoomLevel:99\nfpsCorner:middle\nzoomKey:z\nnoise");
        assert!(!e.zoom);
        assert_eq!(e.zoom_level, 10);
        assert_eq!(e.fps_corner, Corner::TopLeft);
        assert_eq!(e.zoom_key, "key.keyboard.z");
    }

    #[test]
    fn a_mods_zoom_eases_like_the_spyglass() {
        // Camera.tickFov halves the distance to the target each tick.
        let mut camera = crate::camera::Camera::default();
        camera.tick(None, 1.0);
        for _ in 0..30 {
            camera.tick(None, 1.0);
        }
        let target = fov_target(1.0, 4.0);
        camera.tick(None, target);
        let first = camera.calculate_fov(1.0, 70, 1.0, None, Default::default(), false);
        assert!((first - 70.0 * 0.625).abs() < 1e-3, "{first}");
        for _ in 0..30 {
            camera.tick(None, target);
        }
        let settled = camera.calculate_fov(1.0, 70, 1.0, None, Default::default(), false);
        assert!((settled - 70.0 / 4.0).abs() < 1e-3, "{settled}");
        // Deeper than the 0.1 floor stops there, as vanilla clamps.
        for _ in 0..30 {
            camera.tick(None, fov_target(1.0, 20.0));
        }
        let floor = camera.calculate_fov(1.0, 70, 1.0, None, Default::default(), false);
        assert!((floor - 7.0).abs() < 1e-3, "{floor}");
    }

    #[test]
    fn fps_counter_reports_rate_minimum_and_low() {
        let mut c = FpsCounter::default();
        let mut t = 0.0;
        // Two seconds at 100 fps with one 50 ms hitch.
        for i in 0..200 {
            let ms = if i == 150 { 50.0 } else { 10.0 };
            t += f64::from(ms) / 1000.0;
            c.record(t, ms);
        }
        c.computed_at = f64::NEG_INFINITY;
        c.record(t + 0.01, 10.0);
        let r = c.reading();
        assert!((95..=101).contains(&r.fps), "{r:?}");
        assert_eq!(r.min, 20);
        assert_eq!(r.low, 100, "one frame in 200 is under the 1% mark");
        // Hitches beyond 1% of frames pull the 1% low down.
        let mut c = FpsCounter::default();
        let mut t = 0.0;
        for i in 0..100 {
            let ms = if i % 25 == 0 { 40.0 } else { 10.0 };
            t += f64::from(ms) / 1000.0;
            c.record(t, ms);
        }
        c.computed_at = f64::NEG_INFINITY;
        c.record(t, 10.0);
        assert_eq!(c.reading().low, 25);
    }

    #[test]
    fn old_frames_leave_the_window() {
        let mut c = FpsCounter::default();
        c.record(0.0, 100.0);
        for i in 1..=700 {
            c.record(f64::from(i) / 100.0, 10.0);
        }
        assert_eq!(c.reading().min, 100, "the 100 ms frame is 7 s old");
    }

    #[test]
    fn cps_counts_the_last_second() {
        let mut cps = Cps::default();
        for i in 0..10 {
            cps.click(f64::from(i) * 0.1);
        }
        assert_eq!(cps.per_second(0.95), 10);
        assert_eq!(cps.per_second(1.25), 7);
        assert_eq!(cps.per_second(3.0), 0);
    }

    #[test]
    fn items_cycle_through_their_values() {
        let mut e = Extras::default();
        Item::Sort.cycle(&mut e, true);
        assert_eq!(Item::Sort.value(&e), "On");
        // The minimap's old keys still parse, for the move to its mod.
        assert_eq!(
            Extras::parse("minimapSize:7\nminimapZoom:3").minimap_zoom,
            1.0
        );
        for item in rows().into_iter().flatten() {
            assert!(!item.caption().is_empty() && !item.description().is_empty());
        }
    }

    /// The measurement behind `spyglassPolarLook`: how far a spyglass turn moves the picture
    /// sideways at each pitch, off and on, in the camera's own basis at 1920x1080.
    ///
    /// A yaw turn of `d` degrees moves the picture sideways by `d·cos(pitch)` and rolls it
    /// by `d·sin(pitch)`, so vanilla's fixed turn loses the sideways motion near the poles
    /// while its roll grows. `1/cos(pitch)` puts the sideways motion back and leaves the roll
    /// as the only axis that grows, which is why the factor is capped.
    #[test]
    fn the_polar_spyglass_look_keeps_the_sideways_screen_shift() {
        use crate::spyglass_polar_look::{MAX, yaw_scale};
        use horizon_client_input::mouse::{LookOptions, MouseLook, Rotation};

        const W: f64 = 1920.0;
        const H: f64 = 1080.0;
        let aspect = W / H;
        // The spyglass's own field of view: 70 degrees eased toward the 0.1 modifier.
        let mut easing = crate::camera::Camera::default();
        for _ in 0..30 {
            easing.tick(None, 0.1);
        }
        let fov = easing.calculate_fov(1.0, 70, 1.0, None, Default::default(), false);
        assert!((fov - 7.0).abs() < 1e-3, "the spyglass's fov is 7: {fov}");
        // The projection's half-height at one unit out: half the vertical fov.
        let tan_half = f64::from(fov / 2.0).to_radians().tan();

        struct Shift {
            rotation: Rotation,
            lateral_px: f64,
            vertical_px: f64,
            roll_deg: f64,
            forwards: [f32; 3],
        }

        let look = |pitch: f32, on: bool| LookOptions {
            sensitivity: 0.5,
            smooth_camera: false,
            scoping: true,
            invert_x: false,
            invert_y: false,
            yaw_scale: yaw_scale(pitch, on),
        };
        let measure = |pitch: f32, look: LookOptions, d: (f64, f64)| -> Shift {
            let mut before = crate::camera::Camera::default();
            before.set_rotation(0.0, pitch);
            let mut handler = MouseLook::default();
            let mut rotation = Rotation {
                x_rot: pitch,
                y_rot: 0.0,
                x_rot_o: 0.0,
                y_rot_o: 0.0,
            };
            handler.add_motion(d.0, d.1);
            let (xo, yo) = handler
                .handle_accumulated_movement(1.0, true, true, look)
                .expect("a grabbed, active window turns");
            rotation.turn(xo, yo);
            let mut after = crate::camera::Camera::default();
            after.set_rotation(rotation.y_rot, rotation.x_rot);
            let dot = |a: [f32; 3], b: [f32; 3]| {
                f64::from(a[0]) * f64::from(b[0])
                    + f64::from(a[1]) * f64::from(b[1])
                    + f64::from(a[2]) * f64::from(b[2])
            };
            let (forward_0, up_0, forward_1, up_1) =
                (before.forwards, before.up, after.forwards, after.up);
            let right = before.left.map(|c| -c);
            // The direction the camera looks in, from the basis it had before the turn.
            let lateral = dot(forward_1, right).atan2(dot(forward_1, forward_0));
            let vertical = dot(forward_1, up_0).atan2(dot(forward_1, forward_0));
            let roll = dot(up_1, right).atan2(dot(up_1, up_0));
            Shift {
                rotation,
                // Pixels across a 1920 wide screen at the vertical fov, the aspect widening it.
                lateral_px: lateral.tan() / (tan_half * aspect) * (W / 2.0),
                vertical_px: vertical.tan() / tan_half * (H / 2.0),
                roll_deg: roll.to_degrees(),
                forwards: forward_1,
            }
        };

        let pitches = [0.0f32, 45.0, 60.0, 80.0, 82.82, 89.0, 89.9, 90.0];
        let mut table = String::from(
            "pitch | factor | yaw_deg base | yaw_deg on | lateral_px base | lateral_px on | roll_deg base | roll_deg on\n",
        );
        let mut forwards_table = String::from("pitch | forwards base | forwards on\n");
        let mut sideways = Vec::new();
        for pitch in pitches {
            let off = measure(pitch, look(pitch, false), (100.0, 0.0));
            let on = measure(pitch, look(pitch, true), (100.0, 0.0));
            let factor = yaw_scale(pitch, true);
            table += &format!(
                "{pitch} | {factor} | {:.4} | {:.4} | {:.2} | {:.2} | {:.4} | {:.4}\n",
                off.rotation.y_rot,
                on.rotation.y_rot,
                off.lateral_px,
                on.lateral_px,
                off.roll_deg,
                on.roll_deg,
            );
            forwards_table += &format!(
                "{pitch} | {:.4},{:.4},{:.4} | {:.4},{:.4},{:.4}\n",
                off.forwards[0],
                off.forwards[1],
                off.forwards[2],
                on.forwards[0],
                on.forwards[1],
                on.forwards[2],
            );

            // Off, a look built as it was before this option existed - the scale at vanilla's
            // 1.0 - turns exactly the same way, bit for bit.
            let legacy = measure(
                pitch,
                LookOptions {
                    yaw_scale: 1.0,
                    ..look(pitch, false)
                },
                (100.0, 0.0),
            );
            assert_eq!(
                (
                    off.rotation.y_rot.to_bits(),
                    off.rotation.x_rot.to_bits(),
                    off.lateral_px.to_bits(),
                    off.vertical_px.to_bits(),
                    off.roll_deg.to_bits(),
                ),
                (
                    legacy.rotation.y_rot.to_bits(),
                    legacy.rotation.x_rot.to_bits(),
                    legacy.lateral_px.to_bits(),
                    legacy.vertical_px.to_bits(),
                    legacy.roll_deg.to_bits(),
                ),
                "pitch {pitch}: off is vanilla's look"
            );

            // The vertical look keeps vanilla's rate either way: a vertical delta turns no
            // yaw and the same pitch.
            let up_off = measure(pitch, look(pitch, false), (0.0, 100.0));
            let up_on = measure(pitch, look(pitch, true), (0.0, 100.0));
            assert_eq!(up_on.rotation.y_rot.to_bits(), 0.0f32.to_bits());
            assert_eq!(
                up_on.rotation.x_rot.to_bits(),
                up_off.rotation.x_rot.to_bits(),
                "pitch {pitch}: the pitch keeps vanilla's rate"
            );

            if [0.0f32, 45.0, 60.0, 80.0].contains(&pitch) {
                sideways.push((pitch, factor, on.lateral_px));
            }
            if pitch >= 89.9 {
                assert!(!on.lateral_px.is_nan() && on.lateral_px.is_finite());
                assert_eq!(factor, MAX, "pitch {pitch}");
            }
        }
        // On, the sideways motion at 0/45/60/80 degrees is where it is at the horizon.
        let horizon = sideways[0].2;
        for (pitch, factor, px) in sideways {
            let factor_want = if factor == MAX {
                factor
            } else {
                1.0 / f64::from(pitch).to_radians().cos()
            };
            assert!(
                (factor - factor_want).abs() < 1e-9,
                "pitch {pitch}: {factor}"
            );
            assert!(
                (px - horizon).abs() / horizon < 0.01,
                "pitch {pitch}: {px} px sideways against {horizon} at the horizon"
            );
        }
        println!("{table}{forwards_table}");
    }
}
