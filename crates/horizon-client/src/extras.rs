//! Horizon's optional extras (zoom, FPS counter, keystroke overlay): their
//! settings, kept in `horizon-extras.txt` next to `options.txt` so that file
//! stays exactly vanilla's, and the logic behind them. Every extra is off by
//! default; see `docs/extras.md`. Horizon's own switches outside the Extras page
//! (No AI, Agent Control, Discord Rich Presence) are kept here too.
use std::{collections::VecDeque, fs, path::Path};

pub const FILE: &str = "horizon-extras.txt";
/// The name the options screen uses while the zoom key waits for a key.
pub const ZOOM_KEY: &str = "horizon.zoom";
/// The same for the sort key.
pub const SORT_KEY: &str = "horizon.sort";
/// Zoom levels the default cycles through, and the wheel's limits.
pub const ZOOM_LEVELS: [u8; 6] = [2, 3, 4, 6, 8, 10];
pub const ZOOM_MIN: f32 = 1.25;
/// `Camera.tickFov` keeps the field-of-view modifier at 0.1 or more.
pub const ZOOM_MAX: f32 = 10.0;
/// Each wheel step multiplies the zoom by this.
const ZOOM_STEP: f32 = 1.25;

/// The minimap's sizes in GUI pixels, and its zooms in blocks per pixel.
pub const MINIMAP_SIZES: [(u8, &str); 3] = [(64, "Small"), (96, "Medium"), (128, "Large")];
pub const MINIMAP_ZOOMS: [f32; 4] = [0.5, 1.0, 2.0, 4.0];

/// A screen corner for an overlay.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}
impl Corner {
    pub const ALL: [Corner; 4] = [
        Corner::TopLeft,
        Corner::TopRight,
        Corner::BottomLeft,
        Corner::BottomRight,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Corner::TopLeft => "top_left",
            Corner::TopRight => "top_right",
            Corner::BottomLeft => "bottom_left",
            Corner::BottomRight => "bottom_right",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Corner::TopLeft => "Top left",
            Corner::TopRight => "Top right",
            Corner::BottomLeft => "Bottom left",
            Corner::BottomRight => "Bottom right",
        }
    }
    fn parse(s: &str, default: Corner) -> Corner {
        Self::ALL
            .into_iter()
            .find(|c| c.name() == s)
            .unwrap_or(default)
    }
    pub fn top(self) -> bool {
        matches!(self, Corner::TopLeft | Corner::TopRight)
    }
    pub fn left(self) -> bool {
        matches!(self, Corner::TopLeft | Corner::BottomLeft)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Extras {
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
    pub fps: bool,
    pub fps_corner: Corner,
    /// Also show the minimum and 1% low.
    pub fps_lows: bool,
    pub keystrokes: bool,
    pub keystrokes_corner: Corner,
    /// Clicks per second under the mouse buttons.
    pub keystrokes_cps: bool,
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
    /// Mouse Tweaks-style drags in the inventory and storage containers.
    pub mouse_tweaks: bool,
    /// With them, the wheel over a slot moves its items one at a time.
    pub mouse_tweaks_wheel: bool,
    /// Sort buttons on the inventory and chest-like containers.
    pub sort: bool,
    /// A vanilla key name: sorts the part under the cursor.
    pub sort_key: String,
    /// No AI: every AI feature hidden and off (`horizon_core::no_ai`; the launcher writes it too).
    pub no_ai: bool,
    /// Agents may control the game the player starts: it opens its control socket for
    /// horizon-ctl and MCP clients (off by default; an agent's own launch always opens it).
    pub agent_control: bool,
    /// "Playing Horizon" on the player's Discord profile (`discord_presence`; on by default).
    pub discord_presence: bool,
}
impl Default for Extras {
    fn default() -> Self {
        Self {
            fullbright: false,
            dynamic_lights: false,
            zoom: false,
            zoom_key: "key.keyboard.z".into(),
            zoom_level: 4,
            zoom_smooth: true,
            zoom_sensitivity: true,
            fps: false,
            fps_corner: Corner::TopLeft,
            fps_lows: false,
            keystrokes: false,
            keystrokes_corner: Corner::TopLeft,
            keystrokes_cps: true,
            minimap: false,
            minimap_corner: Corner::TopRight,
            minimap_size: 96,
            minimap_round: false,
            minimap_zoom: 1.0,
            minimap_rotate: false,
            auto_reconnect: false,
            mouse_tweaks: false,
            mouse_tweaks_wheel: true,
            sort: false,
            sort_key: "key.keyboard.unknown".into(),
            no_ai: false,
            agent_control: false,
            discord_presence: true,
        }
    }
}
fn bool_text(v: bool) -> &'static str {
    if v { "true" } else { "false" }
}
impl Extras {
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
                "fullbright" => e.fullbright = flag(e.fullbright),
                "dynamicLights" => e.dynamic_lights = flag(e.dynamic_lights),
                "zoom" => e.zoom = flag(e.zoom),
                "zoomKey" if value.starts_with("key.") => e.zoom_key = value.to_owned(),
                "zoomLevel" => {
                    if let Ok(v) = value.parse::<u8>() {
                        e.zoom_level = v.clamp(2, ZOOM_MAX as u8);
                    }
                }
                "zoomSmooth" => e.zoom_smooth = flag(e.zoom_smooth),
                "zoomSensitivity" => e.zoom_sensitivity = flag(e.zoom_sensitivity),
                "fps" => e.fps = flag(e.fps),
                "fpsCorner" => e.fps_corner = Corner::parse(value, e.fps_corner),
                "fpsLows" => e.fps_lows = flag(e.fps_lows),
                "keystrokes" => e.keystrokes = flag(e.keystrokes),
                "keystrokesCorner" => {
                    e.keystrokes_corner = Corner::parse(value, e.keystrokes_corner)
                }
                "keystrokesCps" => e.keystrokes_cps = flag(e.keystrokes_cps),
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
                "mouseTweaks" => e.mouse_tweaks = flag(e.mouse_tweaks),
                "mouseTweaksWheel" => e.mouse_tweaks_wheel = flag(e.mouse_tweaks_wheel),
                "sort" => e.sort = flag(e.sort),
                "sortKey" if value.starts_with("key.") => e.sort_key = value.to_owned(),
                horizon_core::no_ai::KEY => e.no_ai = flag(e.no_ai),
                "agentControl" => e.agent_control = flag(e.agent_control),
                "discordPresence" => e.discord_presence = flag(e.discord_presence),
                _ => {}
            }
        }
        e
    }
    pub fn serialize(&self) -> String {
        format!(
            "fullbright:{}\ndynamicLights:{}\nzoom:{}\nzoomKey:{}\nzoomLevel:{}\nzoomSmooth:{}\nzoomSensitivity:{}\nfps:{}\nfpsCorner:{}\nfpsLows:{}\nkeystrokes:{}\nkeystrokesCorner:{}\nkeystrokesCps:{}\nminimap:{}\nminimapCorner:{}\nminimapSize:{}\nminimapShape:{}\nminimapZoom:{}\nminimapRotate:{}\nautoReconnect:{}\nmouseTweaks:{}\nmouseTweaksWheel:{}\nsort:{}\nsortKey:{}\n{}:{}\nagentControl:{}\ndiscordPresence:{}\n",
            bool_text(self.fullbright),
            bool_text(self.dynamic_lights),
            bool_text(self.zoom),
            self.zoom_key,
            self.zoom_level,
            bool_text(self.zoom_smooth),
            bool_text(self.zoom_sensitivity),
            bool_text(self.fps),
            self.fps_corner.name(),
            bool_text(self.fps_lows),
            bool_text(self.keystrokes),
            self.keystrokes_corner.name(),
            bool_text(self.keystrokes_cps),
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
            bool_text(self.mouse_tweaks),
            bool_text(self.mouse_tweaks_wheel),
            bool_text(self.sort),
            self.sort_key,
            horizon_core::no_ai::KEY,
            bool_text(self.no_ai),
            bool_text(self.agent_control),
            bool_text(self.discord_presence),
        )
    }
    /// Binds the extra key `name` (`ZOOM_KEY`, `SORT_KEY`); false for
    /// another name (a vanilla mapping).
    pub fn set_key(&mut self, name: &str, value: String) -> bool {
        match name {
            ZOOM_KEY => self.zoom_key = value,
            SORT_KEY => self.sort_key = value,
            _ => return false,
        }
        true
    }
    /// Written only once something differs from the defaults, so a vanilla
    /// setup gains no file.
    pub fn save(&self, dir: &Path) {
        let path = dir.join(FILE);
        if *self == Self::default() && !path.exists() {
            return;
        }
        let _ = fs::write(path, self.serialize());
    }
}

/// The zoom key's state: held or not, and the zoom the wheel has set.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Zoom {
    held: bool,
    factor: f32,
}
impl Default for Zoom {
    fn default() -> Self {
        Self {
            held: false,
            factor: 1.0,
        }
    }
}
impl Zoom {
    /// The key went down: start at the default level.
    pub fn press(&mut self, level: u8) {
        self.held = true;
        self.factor = f32::from(level).clamp(ZOOM_MIN, ZOOM_MAX);
    }
    pub fn release(&mut self) {
        self.held = false;
    }
    /// A press or release of the zoom key. Only a press while `playing` (no
    /// screen or text field open) zooms; anywhere else the key belongs to
    /// the screen, and the zoom lets go.
    pub fn key(&mut self, pressed: bool, repeat: bool, playing: bool, level: u8) {
        if !pressed || !playing {
            self.release();
        } else if !repeat {
            self.press(level);
        }
    }
    pub fn held(&self) -> bool {
        self.held
    }
    /// Wheel steps while held (positive zooms in); false when not zooming.
    pub fn scroll(&mut self, steps: f32) -> bool {
        if !self.held {
            return false;
        }
        self.factor = (self.factor * ZOOM_STEP.powf(steps)).clamp(ZOOM_MIN, ZOOM_MAX);
        true
    }
    /// The zoom in effect: 1 when the key is up or the spyglass is in use
    /// (scoping keeps vanilla's spyglass view).
    pub fn factor(&self, scoping: bool) -> f32 {
        if self.held && !scoping {
            self.factor
        } else {
            1.0
        }
    }
}
/// Vanilla key mappings bound to the zoom key. The key still reaches them
/// (a key bound twice triggers both, as vanilla's do), so the Extras page
/// says so.
pub fn zoom_key_conflicts(e: &Extras, keys: &crate::keybinds::KeyBinds) -> Vec<&'static str> {
    if e.zoom_key == "key.keyboard.unknown" {
        return Vec::new();
    }
    crate::keybinds::MAPPINGS
        .iter()
        .map(|m| m.0)
        .filter(|name| keys.get(name) == e.zoom_key)
        .collect()
}
/// The field-of-view modifier target `Camera.tickFov` eases toward: vanilla's
/// (`getFieldOfViewModifier`) divided by the zoom. A zoom of 1 leaves it as is.
pub fn fov_target(vanilla: f32, zoom: f32) -> f32 {
    if zoom == 1.0 { vanilla } else { vanilla / zoom }
}
/// The mouse turning scale while zoomed.
pub fn turn_scale(zoom: f32, lower_sensitivity: bool) -> f64 {
    if lower_sensitivity && zoom > 1.0 {
        1.0 / f64::from(zoom)
    } else {
        1.0
    }
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

/// The settings the Extras page shows, one per row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    Fullbright,
    DynamicLights,
    Zoom,
    ZoomKey,
    ZoomLevel,
    ZoomSmooth,
    ZoomSensitivity,
    Fps,
    FpsCorner,
    FpsLows,
    Keystrokes,
    KeystrokesCorner,
    KeystrokesCps,
    Minimap,
    MinimapCorner,
    MinimapSize,
    MinimapShape,
    MinimapZoom,
    MinimapRotate,
    AutoReconnect,
    MouseTweaks,
    MouseTweaksWheel,
    Sort,
    SortKey,
    /// Not on the Extras page: Horizon Settings shows No AI, its AI Agent page Agent Control.
    NoAi,
    AgentControl,
    /// On the Horizon Settings page.
    DiscordPresence,
}
/// The Extras page: headers and items.
pub fn rows() -> Vec<Result<Item, &'static str>> {
    use Item::*;
    vec![
        Err("Lighting"),
        Ok(Fullbright),
        Ok(DynamicLights),
        Err("Zoom"),
        Ok(Zoom),
        Ok(ZoomKey),
        Ok(ZoomLevel),
        Ok(ZoomSmooth),
        Ok(ZoomSensitivity),
        Err("Minimap"),
        Ok(Minimap),
        Ok(MinimapCorner),
        Ok(MinimapSize),
        Ok(MinimapShape),
        Ok(MinimapZoom),
        Ok(MinimapRotate),
        Err("FPS counter"),
        Ok(Fps),
        Ok(FpsCorner),
        Ok(FpsLows),
        Err("Keystrokes"),
        Ok(Keystrokes),
        Ok(KeystrokesCorner),
        Ok(KeystrokesCps),
        Err("Inventory"),
        Ok(MouseTweaks),
        Ok(MouseTweaksWheel),
        Ok(Sort),
        Ok(SortKey),
        Err("Multiplayer"),
        Ok(AutoReconnect),
    ]
}
/// "1 block", "2 blocks", "½ block".
fn block_count(n: f32) -> String {
    if n < 1.0 {
        "½ block".into()
    } else if n == 1.0 {
        "1 block".into()
    } else {
        format!("{n} blocks")
    }
}
fn next<T: Copy + PartialEq>(all: &[T], current: T, back: bool) -> T {
    let i = all.iter().position(|v| *v == current).unwrap_or(0);
    let n = all.len();
    all[if back { (i + n - 1) % n } else { (i + 1) % n }]
}
impl Item {
    pub fn caption(self) -> &'static str {
        match self {
            Item::Fullbright => "Fullbright",
            Item::DynamicLights => "Dynamic lights",
            Item::Zoom => "Zoom",
            Item::ZoomKey => "Zoom key",
            Item::ZoomLevel => "Starting zoom",
            Item::ZoomSmooth => "Smooth zoom",
            Item::ZoomSensitivity => "Slower turning",
            Item::Fps => "FPS counter",
            Item::FpsCorner | Item::KeystrokesCorner | Item::MinimapCorner => "Corner",
            Item::Minimap => "Minimap",
            Item::MinimapSize => "Size",
            Item::MinimapShape => "Shape",
            Item::MinimapZoom => "Zoom",
            Item::MinimapRotate => "Turn with the player",
            Item::AutoReconnect => "Auto-reconnect",
            Item::MouseTweaks => "Mouse Tweaks controls",
            Item::MouseTweaksWheel => "Scroll wheel moves items",
            Item::Sort => "Sort buttons",
            Item::SortKey => "Sort key",
            Item::FpsLows => "Minimum and 1% low",
            Item::Keystrokes => "Keystrokes",
            Item::KeystrokesCps => "Clicks per second",
            Item::NoAi => "No AI",
            Item::AgentControl => "Agent Control",
            Item::DiscordPresence => "Discord Rich Presence",
        }
    }
    /// The switch this setting belongs to: it does nothing while that is off.
    pub fn parent(self) -> Option<Item> {
        use Item::*;
        Some(match self {
            ZoomKey | ZoomLevel | ZoomSmooth | ZoomSensitivity => Zoom,
            MinimapCorner | MinimapSize | MinimapShape | MinimapZoom | MinimapRotate => Minimap,
            FpsCorner | FpsLows => Fps,
            KeystrokesCorner | KeystrokesCps => Keystrokes,
            MouseTweaksWheel => MouseTweaks,
            SortKey => Sort,
            Fullbright | DynamicLights | Zoom | Minimap | Fps | Keystrokes | MouseTweaks | Sort
            | AutoReconnect | NoAi | AgentControl | DiscordPresence => {
                return None;
            }
        })
    }
    /// What the row says under its caption.
    pub fn description(self) -> &'static str {
        match self {
            Item::Fullbright => "Show blocks and entities at full light, even in dark areas.",
            Item::DynamicLights => {
                "Torches and other light sources held, dropped or on fire light the blocks around them."
            }
            Item::Zoom => "Hold the zoom key to zoom; the wheel changes it while held.",
            Item::ZoomKey => "Click, then press a key. Escape unbinds it.",
            Item::ZoomLevel => "The zoom the key starts at.",
            Item::ZoomSmooth => "Eases in and out like the spyglass.",
            Item::ZoomSensitivity => "Turns slower the more you zoom in.",
            Item::Fps => "Frames per second in a corner of the screen.",
            Item::FpsCorner | Item::KeystrokesCorner | Item::MinimapCorner => {
                "Overlays in one corner stack."
            }
            Item::Minimap => "A map of the loaded terrain around you, in map colours.",
            Item::MinimapSize => "How big the map is on screen.",
            Item::MinimapShape => "Square or round.",
            Item::MinimapZoom => "Blocks per pixel: smaller shows more detail, larger more land.",
            Item::MinimapRotate => "Forward is up; off keeps north up.",
            Item::AutoReconnect => {
                "Rejoins a server after a lost connection or a restart, never after a kick or ban."
            }
            Item::FpsLows => "The slowest frame and the 1% low of the last 5 seconds.",
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
            Item::Keystrokes => "Movement keys, jump and mouse buttons as you press them.",
            Item::KeystrokesCps => "Clicks in the last second under each mouse button.",
            Item::NoAi => {
                "Hides and turns off every AI feature: agent control, horizon-ctl and the MCP servers. The launcher's switch too."
            }
            Item::DiscordPresence => {
                "Shows Playing Horizon on your Discord profile while Discord runs: menus, single player or multiplayer, the dimension and the time played. Never a world's name or a server's address."
            }
            Item::AgentControl => {
                "Your own AI agent can play this game through horizon-ctl or an MCP client, on this computer only."
            }
        }
    }
    /// The frame-time cost the main switch of an extra shows.
    pub fn cost(self) -> Option<crate::graphics_style::Cost> {
        use crate::graphics_style::Cost;
        match self {
            Item::Fullbright | Item::Zoom | Item::Fps | Item::Keystrokes => Some(Cost::None),
            Item::Minimap | Item::DynamicLights => Some(Cost::Low),
            Item::AutoReconnect | Item::MouseTweaks | Item::Sort => Some(Cost::None),
            _ => None,
        }
    }
    /// On/off items and their value.
    pub fn switch(self, e: &Extras) -> Option<bool> {
        Some(match self {
            Item::Fullbright => e.fullbright,
            Item::DynamicLights => e.dynamic_lights,
            Item::Zoom => e.zoom,
            Item::ZoomSmooth => e.zoom_smooth,
            Item::ZoomSensitivity => e.zoom_sensitivity,
            Item::Fps => e.fps,
            Item::FpsLows => e.fps_lows,
            Item::Keystrokes => e.keystrokes,
            Item::KeystrokesCps => e.keystrokes_cps,
            Item::Minimap => e.minimap,
            Item::MinimapRotate => e.minimap_rotate,
            Item::AutoReconnect => e.auto_reconnect,
            Item::MouseTweaks => e.mouse_tweaks,
            Item::MouseTweaksWheel => e.mouse_tweaks_wheel,
            Item::Sort => e.sort,
            Item::NoAi => e.no_ai,
            Item::AgentControl => e.agent_control,
            Item::DiscordPresence => e.discord_presence,
            _ => return None,
        })
    }
    /// The value of the other items, as their button shows it.
    pub fn value(self, e: &Extras) -> String {
        match self {
            Item::ZoomKey => crate::keybinds::display_name(&e.zoom_key).plain(),
            Item::SortKey => crate::keybinds::display_name(&e.sort_key).plain(),
            Item::ZoomLevel => format!("{}x", e.zoom_level),
            Item::FpsCorner => e.fps_corner.label().into(),
            Item::KeystrokesCorner => e.keystrokes_corner.label().into(),
            Item::MinimapCorner => e.minimap_corner.label().into(),
            Item::MinimapSize => MINIMAP_SIZES
                .iter()
                .find(|s| s.0 == e.minimap_size)
                .map_or("Medium", |s| s.1)
                .into(),
            Item::MinimapShape => if e.minimap_round { "Round" } else { "Square" }.into(),
            Item::MinimapZoom => format!("{} per pixel", block_count(e.minimap_zoom)),
            other => match other.switch(e) {
                Some(true) => "On".into(),
                _ => "Off".into(),
            },
        }
    }
    /// A click (`back` for the right button). The zoom key binds instead.
    pub fn cycle(self, e: &mut Extras, back: bool) {
        match self {
            Item::Fullbright => e.fullbright = !e.fullbright,
            Item::DynamicLights => e.dynamic_lights = !e.dynamic_lights,
            Item::Zoom => e.zoom = !e.zoom,
            Item::ZoomSmooth => e.zoom_smooth = !e.zoom_smooth,
            Item::ZoomSensitivity => e.zoom_sensitivity = !e.zoom_sensitivity,
            Item::Fps => e.fps = !e.fps,
            Item::FpsLows => e.fps_lows = !e.fps_lows,
            Item::Keystrokes => e.keystrokes = !e.keystrokes,
            Item::KeystrokesCps => e.keystrokes_cps = !e.keystrokes_cps,
            Item::ZoomLevel => e.zoom_level = next(&ZOOM_LEVELS, e.zoom_level, back),
            Item::FpsCorner => e.fps_corner = next(&Corner::ALL, e.fps_corner, back),
            Item::KeystrokesCorner => {
                e.keystrokes_corner = next(&Corner::ALL, e.keystrokes_corner, back)
            }
            Item::Minimap => e.minimap = !e.minimap,
            Item::MinimapRotate => e.minimap_rotate = !e.minimap_rotate,
            Item::AutoReconnect => e.auto_reconnect = !e.auto_reconnect,
            Item::MouseTweaks => e.mouse_tweaks = !e.mouse_tweaks,
            Item::MouseTweaksWheel => e.mouse_tweaks_wheel = !e.mouse_tweaks_wheel,
            Item::Sort => e.sort = !e.sort,
            Item::MinimapCorner => e.minimap_corner = next(&Corner::ALL, e.minimap_corner, back),
            Item::MinimapSize => {
                let sizes: Vec<u8> = MINIMAP_SIZES.iter().map(|s| s.0).collect();
                e.minimap_size = next(&sizes, e.minimap_size, back);
            }
            Item::MinimapShape => e.minimap_round = !e.minimap_round,
            Item::MinimapZoom => e.minimap_zoom = next(&MINIMAP_ZOOMS, e.minimap_zoom, back),
            Item::NoAi => e.no_ai = !e.no_ai,
            Item::AgentControl => e.agent_control = !e.agent_control,
            Item::DiscordPresence => e.discord_presence = !e.discord_presence,
            Item::ZoomKey | Item::SortKey => {}
        }
    }
    /// The key name the options screen waits on for a key item.
    pub fn key_name(self) -> Option<&'static str> {
        match self {
            Item::ZoomKey => Some(ZOOM_KEY),
            Item::SortKey => Some(SORT_KEY),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn everything_is_off_by_default_and_round_trips() {
        let d = Extras::default();
        assert!(!d.fullbright && !d.dynamic_lights);
        assert!(!d.zoom && !d.fps && !d.keystrokes && !d.minimap && !d.auto_reconnect);
        assert!(!d.mouse_tweaks && !d.sort);
        assert!(d.discord_presence);
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
        e.minimap = true;
        e.minimap_size = 128;
        e.minimap_round = true;
        e.minimap_zoom = 0.5;
        e.minimap_rotate = true;
        e.minimap_corner = Corner::BottomLeft;
        e.auto_reconnect = true;
        e.mouse_tweaks = true;
        e.mouse_tweaks_wheel = false;
        e.sort = true;
        e.sort_key = "key.keyboard.r".into();
        e.discord_presence = false;
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
    fn default_zoom_key_is_free_in_vanilla() {
        let key = Extras::default().zoom_key;
        assert!(crate::keybinds::key_code(&key).is_some());
        assert!(
            crate::keybinds::MAPPINGS.iter().all(|m| m.1 != key),
            "{key} is a vanilla binding"
        );
    }

    #[test]
    fn zoom_is_vanilla_unless_held_and_not_scoping() {
        let mut z = Zoom::default();
        assert_eq!(z.factor(false), 1.0);
        assert_eq!(fov_target(1.1, z.factor(false)), 1.1);
        assert!(!z.scroll(1.0), "the wheel is the hotbar's when not zooming");
        z.press(4);
        assert_eq!(z.factor(false), 4.0);
        assert_eq!(z.factor(true), 1.0, "the spyglass wins");
        assert!((fov_target(1.0, 4.0) - 0.25).abs() < 1e-6);
        assert!(z.scroll(1.0));
        assert!((z.factor(false) - 5.0).abs() < 1e-5);
        for _ in 0..40 {
            z.scroll(1.0);
        }
        assert_eq!(z.factor(false), ZOOM_MAX);
        for _ in 0..80 {
            z.scroll(-1.0);
        }
        assert_eq!(z.factor(false), ZOOM_MIN);
        z.release();
        assert_eq!(z.factor(false), 1.0);
        z.press(2);
        assert_eq!(z.factor(false), 2.0, "each press starts at the default");
    }

    #[test]
    fn zoom_target_eases_like_the_spyglass() {
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
    fn the_zoom_key_only_zooms_while_playing() {
        let mut z = Zoom::default();
        // In chat, a sign, a text field or any screen: no zoom.
        z.key(true, false, false, 4);
        assert!(!z.held());
        z.key(true, false, true, 4);
        assert!(z.held());
        // A screen opening while held lets go (its key presses are the screen's).
        z.key(true, true, false, 4);
        assert!(!z.held());
        // Key repeats do not restart a zoom the wheel changed.
        z.key(true, false, true, 4);
        z.scroll(1.0);
        z.key(true, true, true, 4);
        assert!((z.factor(false) - 5.0).abs() < 1e-5);
        z.key(false, false, true, 4);
        assert!(!z.held());
    }

    #[test]
    fn zoom_key_conflicts_name_the_vanilla_mappings() {
        let mut e = Extras::default();
        let mut keys = crate::keybinds::KeyBinds::default();
        assert!(zoom_key_conflicts(&e, &keys).is_empty());
        e.zoom_key = "key.keyboard.c".into();
        assert_eq!(
            zoom_key_conflicts(&e, &keys),
            [
                "key.saveToolbarActivator",
                "key.debug.crash",
                "key.debug.copyLocation"
            ]
        );
        keys.set("key.drop", "key.keyboard.z");
        e.zoom_key = "key.keyboard.z".into();
        assert_eq!(zoom_key_conflicts(&e, &keys), ["key.drop"]);
        e.zoom_key = "key.keyboard.unknown".into();
        keys.set("key.smoothCamera", "key.keyboard.unknown");
        assert!(zoom_key_conflicts(&e, &keys).is_empty());
    }

    #[test]
    fn sensitivity_scales_with_zoom() {
        assert_eq!(turn_scale(1.0, true), 1.0);
        assert_eq!(turn_scale(4.0, false), 1.0);
        assert_eq!(turn_scale(4.0, true), 0.25);
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
        Item::ZoomLevel.cycle(&mut e, false);
        assert_eq!(e.zoom_level, 6);
        Item::ZoomLevel.cycle(&mut e, true);
        Item::ZoomLevel.cycle(&mut e, true);
        assert_eq!(e.zoom_level, 3);
        Item::FpsCorner.cycle(&mut e, true);
        assert_eq!(e.fps_corner, Corner::BottomRight);
        Item::Zoom.cycle(&mut e, false);
        assert_eq!(Item::Zoom.switch(&e), Some(true));
        assert_eq!(Item::ZoomKey.value(&e), "Z");
        Item::MinimapZoom.cycle(&mut e, true);
        assert_eq!(Item::MinimapZoom.value(&e), "½ block per pixel");
        Item::MinimapSize.cycle(&mut e, false);
        assert_eq!(Item::MinimapSize.value(&e), "Large");
        assert_eq!(
            Extras::parse("minimapSize:7\nminimapZoom:3").minimap_zoom,
            1.0
        );
        for item in rows().into_iter().flatten() {
            assert!(!item.caption().is_empty() && !item.description().is_empty());
        }
    }
}
