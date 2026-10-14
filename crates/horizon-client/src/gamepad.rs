//! Controllers: Horizon's own feature (vanilla 26.3 has no gamepad input).
//! The default layout is Bedrock Edition's; see docs/controllers.md.
//!
//! This module is the backend-free part: positional buttons, the action
//! bindings, deadzones and the look curve, button edges from successive pad
//! states, and `horizon-controller.txt`, kept next to `options.txt` so that
//! file stays exactly vanilla's. The game reads the pads with gilrs.
use std::{collections::BTreeMap, fs, path::Path};

pub const FILE: &str = "horizon-controller.txt";

/// A button by its position on the pad (gilrs' and SDL's layout): `South`
/// is Xbox A, PlayStation ✕, Nintendo B.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Button {
    South,
    East,
    West,
    North,
    LeftBumper,
    RightBumper,
    LeftTrigger,
    RightTrigger,
    Select,
    Start,
    LeftStick,
    RightStick,
    DPadUp,
    DPadDown,
    DPadLeft,
    DPadRight,
}
impl Button {
    pub const ALL: [Button; 16] = [
        Button::South,
        Button::East,
        Button::West,
        Button::North,
        Button::LeftBumper,
        Button::RightBumper,
        Button::LeftTrigger,
        Button::RightTrigger,
        Button::Select,
        Button::Start,
        Button::LeftStick,
        Button::RightStick,
        Button::DPadUp,
        Button::DPadDown,
        Button::DPadLeft,
        Button::DPadRight,
    ];
    /// The name in `horizon-controller.txt` and `horizon-ctl pad`.
    pub fn name(self) -> &'static str {
        match self {
            Button::South => "south",
            Button::East => "east",
            Button::West => "west",
            Button::North => "north",
            Button::LeftBumper => "lb",
            Button::RightBumper => "rb",
            Button::LeftTrigger => "lt",
            Button::RightTrigger => "rt",
            Button::Select => "select",
            Button::Start => "start",
            Button::LeftStick => "ls",
            Button::RightStick => "rs",
            Button::DPadUp => "up",
            Button::DPadDown => "down",
            Button::DPadLeft => "left",
            Button::DPadRight => "right",
        }
    }
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|b| b.name() == name)
    }
    fn bit(self) -> u32 {
        1 << self as u32
    }
    /// What is printed on the button for a pad family (`Auto` reads as Xbox).
    pub fn label(self, style: GlyphStyle) -> &'static str {
        let i = self as usize;
        match style {
            GlyphStyle::PlayStation => [
                "×", "○", "□", "△", "L1", "R1", "L2", "R2", "Create", "Options", "L3", "R3", "↑",
                "↓", "←", "→",
            ][i],
            GlyphStyle::Nintendo => [
                "B", "A", "Y", "X", "L", "R", "ZL", "ZR", "−", "+", "LS", "RS", "↑", "↓", "←", "→",
            ][i],
            GlyphStyle::Xbox | GlyphStyle::Auto => [
                "A", "B", "X", "Y", "LB", "RB", "LT", "RT", "View", "Menu", "LS", "RS", "↑", "↓",
                "←", "→",
            ][i],
        }
    }
    /// The glyph's colour (RGB): face buttons in their family's colours,
    /// the rest white.
    pub fn color(self, style: GlyphStyle) -> [u8; 3] {
        let face = match self {
            Button::South => 0,
            Button::East => 1,
            Button::West => 2,
            Button::North => 3,
            _ => return [255, 255, 255],
        };
        match style {
            GlyphStyle::PlayStation => [
                [124, 178, 232],
                [235, 110, 110],
                [222, 145, 205],
                [92, 205, 170],
            ][face],
            GlyphStyle::Nintendo => [255, 255, 255],
            GlyphStyle::Xbox | GlyphStyle::Auto => [
                [110, 200, 90],
                [235, 90, 80],
                [80, 150, 240],
                [240, 200, 60],
            ][face],
        }
    }
}

/// A pad's inputs at one moment. Sticks are `[x, y]` with right and up
/// positive; triggers run 0..1.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PadState {
    /// Digital buttons as a bit set (triggers are in `triggers`).
    pub buttons: u32,
    pub left: [f32; 2],
    pub right: [f32; 2],
    pub triggers: [f32; 2],
}
impl PadState {
    pub fn set(&mut self, button: Button, down: bool) {
        if down {
            self.buttons |= button.bit();
        } else {
            self.buttons &= !button.bit();
        }
    }
    pub fn held(&self, button: Button) -> bool {
        self.buttons & button.bit() != 0
    }
    /// Two sources at once (a real pad and `horizon-ctl pad`): buttons held on
    /// either, sticks and triggers added.
    pub fn merge(&self, other: &PadState) -> PadState {
        let add = |a: [f32; 2], b: [f32; 2]| {
            [
                (a[0] + b[0]).clamp(-1.0, 1.0),
                (a[1] + b[1]).clamp(-1.0, 1.0),
            ]
        };
        PadState {
            buttons: self.buttons | other.buttons,
            left: add(self.left, other.left),
            right: add(self.right, other.right),
            triggers: [
                (self.triggers[0] + other.triggers[0]).clamp(0.0, 1.0),
                (self.triggers[1] + other.triggers[1]).clamp(0.0, 1.0),
            ],
        }
    }
    /// Anything pushed past the deadzones (the pad is in use).
    pub fn active(&self, settings: &PadSettings) -> bool {
        self.buttons != 0
            || length(self.left) > settings.move_deadzone
            || length(self.right) > settings.look_deadzone
            || self
                .triggers
                .iter()
                .any(|t| *t >= settings.trigger_threshold)
    }
}

/// Button presses and releases between successive states. Triggers count as
/// buttons past `threshold`, with a little hysteresis so a trigger resting
/// near the threshold doesn't chatter.
#[derive(Clone, Debug, Default)]
pub struct Edges {
    held: u32,
}
impl Edges {
    pub fn update(&mut self, state: &PadState, threshold: f32) -> Vec<(Button, bool)> {
        let mut now = state.buttons;
        for (trigger, button) in [(0, Button::LeftTrigger), (1, Button::RightTrigger)] {
            let was = self.held & button.bit() != 0;
            let value = state.triggers[trigger];
            let on = if was {
                value >= (threshold - 0.1).max(0.05)
            } else {
                value >= threshold
            };
            if on {
                now |= button.bit();
            } else {
                now &= !button.bit();
            }
        }
        let changed = now ^ self.held;
        self.held = now;
        Button::ALL
            .into_iter()
            .filter(|b| changed & b.bit() != 0)
            .map(|b| (b, now & b.bit() != 0))
            .collect()
    }
    /// Release everything (the pad was unplugged or turned off).
    pub fn release_all(&mut self) -> Vec<(Button, bool)> {
        let held = std::mem::take(&mut self.held);
        Button::ALL
            .into_iter()
            .filter(|b| held & b.bit() != 0)
            .map(|b| (b, false))
            .collect()
    }
    pub fn is_held(&self, button: Button) -> bool {
        self.held & button.bit() != 0
    }
}

/// Horizon's pad-only actions (no vanilla key mapping).
pub const HOTBAR_PREVIOUS: &str = "pad.hotbarPrevious";
pub const HOTBAR_NEXT: &str = "pad.hotbarNext";

/// Bindable actions and their default buttons: vanilla key mappings (the
/// game acts as their default key) and Horizon's pad-only actions. Start is
/// always pause / back and isn't listed.
pub const ACTIONS: &[(&str, Option<Button>)] = &[
    ("key.jump", Some(Button::South)),
    ("key.drop", Some(Button::East)),
    ("key.swapOffhand", Some(Button::West)),
    ("key.inventory", Some(Button::North)),
    (HOTBAR_PREVIOUS, Some(Button::LeftBumper)),
    (HOTBAR_NEXT, Some(Button::RightBumper)),
    ("key.use", Some(Button::LeftTrigger)),
    ("key.attack", Some(Button::RightTrigger)),
    ("key.sprint", Some(Button::LeftStick)),
    ("key.sneak", Some(Button::RightStick)),
    ("key.togglePerspective", Some(Button::DPadUp)),
    ("key.pickItem", Some(Button::DPadLeft)),
    ("key.chat", Some(Button::DPadRight)),
    ("key.advancements", Some(Button::DPadDown)),
    ("key.playerlist", Some(Button::Select)),
    ("key.command", None),
    ("key.screenshot", None),
    ("key.toggleGui", None),
    ("key.smoothCamera", None),
];

/// An action's name in the binds list: vanilla's translation key, or
/// Horizon's own English for the pad-only actions.
pub fn action_caption(action: &str) -> &str {
    match action {
        HOTBAR_PREVIOUS => "Hotbar: Previous Slot",
        HOTBAR_NEXT => "Hotbar: Next Slot",
        other => other,
    }
}

/// Which button pictures prompts use.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GlyphStyle {
    /// From the pad's maker.
    #[default]
    Auto,
    Xbox,
    PlayStation,
    Nintendo,
}
impl GlyphStyle {
    pub fn name(self) -> &'static str {
        match self {
            GlyphStyle::Auto => "auto",
            GlyphStyle::Xbox => "xbox",
            GlyphStyle::PlayStation => "playstation",
            GlyphStyle::Nintendo => "nintendo",
        }
    }
    pub fn parse(name: &str) -> Self {
        match name {
            "xbox" => GlyphStyle::Xbox,
            "playstation" => GlyphStyle::PlayStation,
            "nintendo" => GlyphStyle::Nintendo,
            _ => GlyphStyle::Auto,
        }
    }
    /// The style the player chose, else the connected pad's.
    pub fn resolve(self, detected: GlyphStyle) -> GlyphStyle {
        match self {
            GlyphStyle::Auto => match detected {
                GlyphStyle::Auto => GlyphStyle::Xbox,
                d => d,
            },
            chosen => chosen,
        }
    }
    pub const ALL: [GlyphStyle; 4] = [
        GlyphStyle::Auto,
        GlyphStyle::Xbox,
        GlyphStyle::PlayStation,
        GlyphStyle::Nintendo,
    ];
    /// The style for a pad by its USB vendor (Sony, Nintendo; Microsoft,
    /// Valve's Steam Deck and Steam Input's virtual pad, and everything else
    /// look like Xbox).
    pub fn of_vendor(vendor: Option<u16>) -> Self {
        match vendor {
            Some(0x054c) => GlyphStyle::PlayStation,
            Some(0x057e) => GlyphStyle::Nintendo,
            _ => GlyphStyle::Xbox,
        }
    }
}

/// The controller settings (`horizon-controller.txt`).
#[derive(Clone, Debug, PartialEq)]
pub struct PadSettings {
    pub enabled: bool,
    pub vibration: bool,
    /// 0..1; 0.5 is the default speed, 1 twice that.
    pub look_sensitivity: f32,
    pub invert_x: bool,
    pub invert_y: bool,
    pub move_deadzone: f32,
    pub look_deadzone: f32,
    pub trigger_threshold: f32,
    /// The sneak button flips sneaking (Bedrock's default) instead of
    /// holding it, unless vanilla's toggle crouch is on already.
    pub toggle_sneak: bool,
    pub glyphs: GlyphStyle,
    /// Action name -> button, `None` for unbound; missing means the default.
    bindings: BTreeMap<&'static str, Option<Button>>,
}
impl Default for PadSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            vibration: true,
            look_sensitivity: 0.5,
            invert_x: false,
            invert_y: false,
            move_deadzone: 0.15,
            look_deadzone: 0.12,
            trigger_threshold: 0.4,
            toggle_sneak: true,
            glyphs: GlyphStyle::Auto,
            bindings: BTreeMap::new(),
        }
    }
}
fn bool_text(v: bool) -> &'static str {
    if v { "true" } else { "false" }
}
impl PadSettings {
    pub fn load(dir: &Path) -> Self {
        fs::read_to_string(dir.join(FILE))
            .map(|t| Self::parse(&t))
            .unwrap_or_default()
    }
    /// Written once something differs from the defaults, so players who
    /// never touch a controller get no new file.
    pub fn save(&self, dir: &Path) {
        let path = dir.join(FILE);
        if *self != Self::default() || path.exists() {
            let _ = fs::write(path, self.serialize());
        }
    }
    pub fn parse(text: &str) -> Self {
        let mut s = Self::default();
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
            let unit = |d: f32| {
                value
                    .parse::<f32>()
                    .ok()
                    .filter(|v| v.is_finite())
                    .map_or(d, |v| v.clamp(0.0, 1.0))
            };
            match key.trim() {
                "enabled" => s.enabled = flag(s.enabled),
                "vibration" => s.vibration = flag(s.vibration),
                "lookSensitivity" => s.look_sensitivity = unit(s.look_sensitivity),
                "invertX" => s.invert_x = flag(s.invert_x),
                "invertY" => s.invert_y = flag(s.invert_y),
                "moveDeadzone" => s.move_deadzone = unit(s.move_deadzone).min(0.9),
                "lookDeadzone" => s.look_deadzone = unit(s.look_deadzone).min(0.9),
                "triggerThreshold" => {
                    s.trigger_threshold = unit(s.trigger_threshold).clamp(0.1, 0.9)
                }
                "toggleSneak" => s.toggle_sneak = flag(s.toggle_sneak),
                "glyphs" => s.glyphs = GlyphStyle::parse(value),
                other => {
                    if let Some(action) = other.strip_prefix("bind_") {
                        let button = if value == "none" {
                            Some(None)
                        } else {
                            Button::parse(value).map(Some)
                        };
                        if let Some(button) = button {
                            s.bind(action, button);
                        }
                    }
                }
            }
        }
        s
    }
    pub fn serialize(&self) -> String {
        let mut text = format!(
            "enabled:{}\nvibration:{}\nlookSensitivity:{}\ninvertX:{}\ninvertY:{}\nmoveDeadzone:{}\nlookDeadzone:{}\ntriggerThreshold:{}\ntoggleSneak:{}\nglyphs:{}\n",
            bool_text(self.enabled),
            bool_text(self.vibration),
            self.look_sensitivity,
            bool_text(self.invert_x),
            bool_text(self.invert_y),
            self.move_deadzone,
            self.look_deadzone,
            self.trigger_threshold,
            bool_text(self.toggle_sneak),
            self.glyphs.name(),
        );
        for (action, button) in &self.bindings {
            text.push_str(&format!(
                "bind_{action}:{}\n",
                button.map_or("none", Button::name)
            ));
        }
        text
    }
    pub fn default_of(action: &str) -> Option<Button> {
        ACTIONS.iter().find(|a| a.0 == action).and_then(|a| a.1)
    }
    /// The button an action is bound to.
    pub fn binding(&self, action: &str) -> Option<Button> {
        match self.bindings.get(action) {
            Some(button) => *button,
            None => Self::default_of(action),
        }
    }
    /// Bind an action (`None` unbinds); unknown actions are ignored.
    pub fn bind(&mut self, action: &str, button: Option<Button>) {
        let Some((name, default)) = ACTIONS.iter().find(|a| a.0 == action) else {
            return;
        };
        if button == *default {
            self.bindings.remove(name);
        } else {
            self.bindings.insert(name, button);
        }
    }
    pub fn reset_bindings(&mut self) {
        self.bindings.clear();
    }
    /// Other actions on the same button (marked in the binds list).
    pub fn conflicts(&self, action: &str) -> bool {
        self.binding(action)
            .is_some_and(|b| self.actions_of(b).iter().any(|a| *a != action))
    }
    pub fn is_default(&self, action: &str) -> bool {
        !self.bindings.contains_key(action)
    }
    /// The actions a button triggers, in `ACTIONS` order.
    pub fn actions_of(&self, button: Button) -> Vec<&'static str> {
        ACTIONS
            .iter()
            .map(|a| a.0)
            .filter(|a| self.binding(a) == Some(button))
            .collect()
    }
}

fn length(v: [f32; 2]) -> f32 {
    (v[0] * v[0] + v[1] * v[1]).sqrt()
}

/// A radial deadzone: inside it the stick is centred; beyond it the rest of
/// the travel is rescaled to 0..1 in the same direction, so diagonals aren't
/// clipped and the first movement past the deadzone is small.
pub fn radial(v: [f32; 2], deadzone: f32) -> [f32; 2] {
    let len = length(v);
    if !len.is_finite() || len <= deadzone || len == 0.0 {
        return [0.0, 0.0];
    }
    let scaled = ((len - deadzone) / (1.0 - deadzone)).min(1.0);
    [v[0] / len * scaled, v[1] / len * scaled]
}

/// The left stick as the player's move vector, in `KeyboardInput`'s layout
/// (`[left, forward]`, length at most 1); `None` inside the deadzone.
pub fn move_vector(stick: [f32; 2], deadzone: f32) -> Option<[f32; 2]> {
    let v = radial(stick, deadzone);
    (v != [0.0, 0.0]).then_some([-v[0], v[1]])
}

/// Degrees per second at full tilt and the default sensitivity.
pub const YAW_SPEED: f32 = 200.0;
pub const PITCH_SPEED: f32 = 140.0;

/// The view turn from the right stick over `seconds`: `(yaw, pitch)` in
/// degrees, pitch positive looking down as in `Entity.turn`. A squared
/// curve keeps small tilts precise; a spyglass slows it down.
pub fn look_delta(
    stick: [f32; 2],
    settings: &PadSettings,
    seconds: f32,
    scoping: bool,
) -> (f32, f32) {
    let v = radial(stick, settings.look_deadzone);
    let len = length(v);
    if len == 0.0 {
        return (0.0, 0.0);
    }
    // The vector already has length `len`: scaling by it squares the tilt.
    let curve = len;
    let speed = settings.look_sensitivity * 2.0 * if scoping { 0.25 } else { 1.0 };
    let x = v[0] * curve * if settings.invert_x { -1.0 } else { 1.0 };
    let y = v[1] * curve * if settings.invert_y { -1.0 } else { 1.0 };
    (
        x * YAW_SPEED * speed * seconds,
        -y * PITCH_SPEED * speed * seconds,
    )
}

/// The hotbar slot after a bumper: one step with wrap-around.
pub fn hotbar_step(selected: usize, forward: bool) -> usize {
    if forward {
        (selected + 1) % 9
    } else {
        (selected + 8) % 9
    }
}

/// Menu cursor speed in GUI pixels per second at full tilt.
pub const CURSOR_SPEED: f32 = 300.0;

/// How far the menu cursor moves for a stick over `seconds`, in GUI pixels
/// (`y` down): a squared curve, so a light touch lands on a slot.
pub fn cursor_delta(stick: [f32; 2], deadzone: f32, seconds: f32) -> (f32, f32) {
    let v = radial(stick, deadzone);
    let len = length(v);
    if len == 0.0 {
        return (0.0, 0.0);
    }
    let speed = (CURSOR_SPEED * len + 30.0) * seconds;
    (v[0] * speed, -v[1] * speed)
}

/// The target a D-pad press moves focus to: the nearest one inside a cone
/// around the direction, weighing sideways distance double.
pub fn focus_step(from: (f32, f32), button: Button, targets: &[(f32, f32)]) -> Option<usize> {
    let (dx, dy) = match button {
        Button::DPadUp => (0.0, -1.0),
        Button::DPadDown => (0.0, 1.0),
        Button::DPadLeft => (-1.0, 0.0),
        Button::DPadRight => (1.0, 0.0),
        _ => return None,
    };
    targets
        .iter()
        .enumerate()
        .filter_map(|(i, t)| {
            let (x, y) = (t.0 - from.0, t.1 - from.1);
            let along = x * dx + y * dy;
            let across = (x * dy - y * dx).abs();
            (along > 2.0 && across <= along * 2.0).then_some((i, along + 2.0 * across))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

/// The target within `radius` of `at`, nearest first (the cursor settles
/// onto a slot or button when the stick lets go).
pub fn nearest(at: (f32, f32), targets: &[(f32, f32)], radius: f32) -> Option<usize> {
    targets
        .iter()
        .enumerate()
        .map(|(i, t)| (i, (t.0 - at.0).hypot(t.1 - at.1)))
        .filter(|(_, d)| *d <= radius)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_follow_bedrock() {
        let s = PadSettings::default();
        assert_eq!(s.actions_of(Button::South), ["key.jump"]);
        assert_eq!(s.actions_of(Button::RightTrigger), ["key.attack"]);
        assert_eq!(s.actions_of(Button::LeftTrigger), ["key.use"]);
        assert_eq!(s.actions_of(Button::North), ["key.inventory"]);
        assert_eq!(s.actions_of(Button::RightBumper), [HOTBAR_NEXT]);
        assert!(s.actions_of(Button::Start).is_empty());
        // Every default action names a vanilla mapping or a pad action.
        for (action, _) in ACTIONS {
            assert!(
                action.starts_with("pad.")
                    || crate::keybinds::MAPPINGS.iter().any(|m| m.0 == *action),
                "{action}"
            );
        }
    }

    #[test]
    fn settings_round_trip_and_store_only_changes() {
        let mut s = PadSettings::default();
        assert!(!s.serialize().contains("bind_"));
        s.bind("key.jump", Some(Button::East));
        s.bind("key.drop", None);
        s.bind("key.sneak", Some(Button::RightStick));
        s.invert_y = true;
        s.look_sensitivity = 0.75;
        s.glyphs = GlyphStyle::PlayStation;
        let text = s.serialize();
        assert!(text.contains("bind_key.jump:east\n"));
        assert!(text.contains("bind_key.drop:none\n"));
        assert!(!text.contains("key.sneak"));
        let back = PadSettings::parse(&text);
        assert_eq!(back, s);
        assert_eq!(back.binding("key.drop"), None);
        assert_eq!(back.actions_of(Button::East), ["key.jump"]);
        // Garbage keeps the defaults.
        let odd =
            PadSettings::parse("lookSensitivity:NaN\nbind_key.nope:south\nbind_key.jump:x9\n");
        assert_eq!(odd, PadSettings::default());
    }

    #[test]
    fn radial_deadzone_rescales() {
        assert_eq!(radial([0.1, 0.05], 0.15), [0.0, 0.0]);
        let full = radial([1.0, 0.0], 0.15);
        assert!((full[0] - 1.0).abs() < 1e-6);
        let half = radial([0.0, 0.575], 0.15);
        assert!((half[1] - 0.5).abs() < 1e-6);
        // A worn stick reporting more than 1 still caps at 1.
        let over = radial([1.0, 1.0], 0.15);
        assert!((length(over) - 1.0).abs() < 1e-6);
        assert_eq!(move_vector([0.0, 0.0], 0.15), None);
        let m = move_vector([1.0, 0.0], 0.15).unwrap();
        assert!(m[0] < -0.99 && m[1] == 0.0, "right is negative left");
    }

    #[test]
    fn look_turns_right_and_up() {
        let s = PadSettings::default();
        let (yaw, pitch) = look_delta([1.0, 0.0], &s, 1.0, false);
        assert!((yaw - YAW_SPEED).abs() < 1e-3 && pitch == 0.0);
        let (_, pitch) = look_delta([0.0, 1.0], &s, 0.5, false);
        assert!((pitch + PITCH_SPEED * 0.5).abs() < 1e-3, "up looks up");
        let inverted = PadSettings {
            invert_y: true,
            ..PadSettings::default()
        };
        assert!(look_delta([0.0, 1.0], &inverted, 1.0, false).1 > 0.0);
        let (small, _) = look_delta([0.5, 0.0], &s, 1.0, false);
        assert!(
            small < YAW_SPEED * 0.5,
            "the curve is gentler near the centre"
        );
        assert_eq!(look_delta([0.05, 0.05], &s, 1.0, false), (0.0, 0.0));
        assert!(look_delta([1.0, 0.0], &s, 1.0, true).0 < YAW_SPEED * 0.5);
    }

    #[test]
    fn edges_with_trigger_hysteresis() {
        let mut edges = Edges::default();
        let mut state = PadState::default();
        state.set(Button::South, true);
        assert_eq!(edges.update(&state, 0.4), [(Button::South, true)]);
        assert!(edges.update(&state, 0.4).is_empty());
        state.triggers[1] = 0.45;
        assert_eq!(edges.update(&state, 0.4), [(Button::RightTrigger, true)]);
        state.triggers[1] = 0.35;
        assert!(
            edges.update(&state, 0.4).is_empty(),
            "inside the hysteresis"
        );
        state.triggers[1] = 0.2;
        state.set(Button::South, false);
        assert_eq!(
            edges.update(&state, 0.4),
            [(Button::South, false), (Button::RightTrigger, false)]
        );
        state.set(Button::DPadUp, true);
        edges.update(&state, 0.4);
        assert_eq!(edges.release_all(), [(Button::DPadUp, false)]);
    }

    #[test]
    fn merge_and_activity() {
        let s = PadSettings::default();
        let mut a = PadState::default();
        assert!(!a.active(&s));
        a.left = [0.1, 0.0];
        assert!(!a.active(&s), "drift inside the deadzone");
        let mut b = PadState::default();
        b.set(Button::North, true);
        b.left = [0.95, 0.0];
        let m = a.merge(&b);
        assert!(m.held(Button::North) && m.left[0] == 1.0 && m.active(&s));
    }

    #[test]
    fn focus_moves_between_slots() {
        // A 3x2 grid of slots 18 apart and a button below it.
        let mut targets: Vec<(f32, f32)> = (0..6)
            .map(|i| (8.0 + (i % 3) as f32 * 18.0, 8.0 + (i / 3) as f32 * 18.0))
            .collect();
        targets.push((26.0, 80.0));
        let from = targets[0];
        assert_eq!(focus_step(from, Button::DPadRight, &targets), Some(1));
        assert_eq!(focus_step(from, Button::DPadDown, &targets), Some(3));
        assert_eq!(focus_step(from, Button::DPadUp, &targets), None);
        assert_eq!(focus_step(from, Button::DPadLeft, &targets), None);
        assert_eq!(focus_step(targets[4], Button::DPadDown, &targets), Some(6));
        // The far end of a row skips nothing in between.
        assert_eq!(focus_step(targets[2], Button::DPadLeft, &targets), Some(1));
        assert_eq!(nearest((12.0, 10.0), &targets, 9.0), Some(0));
        assert_eq!(nearest((100.0, 100.0), &targets, 9.0), None);
        let (x, y) = cursor_delta([0.0, 1.0], 0.15, 0.1);
        assert!(x == 0.0 && y < -30.0, "up moves up the screen");
        assert_eq!(cursor_delta([0.1, 0.0], 0.15, 0.1), (0.0, 0.0));
    }

    #[test]
    fn labels_and_conflicts() {
        assert_eq!(Button::South.label(GlyphStyle::Xbox), "A");
        assert_eq!(Button::South.label(GlyphStyle::PlayStation), "×");
        assert_eq!(Button::South.label(GlyphStyle::Nintendo), "B");
        assert_eq!(Button::RightTrigger.label(GlyphStyle::PlayStation), "R2");
        assert_eq!(Button::DPadLeft.label(GlyphStyle::Xbox), "←");
        assert_eq!(
            GlyphStyle::Auto.resolve(GlyphStyle::PlayStation),
            GlyphStyle::PlayStation
        );
        assert_eq!(
            GlyphStyle::Xbox.resolve(GlyphStyle::PlayStation),
            GlyphStyle::Xbox
        );
        assert_eq!(GlyphStyle::Auto.resolve(GlyphStyle::Auto), GlyphStyle::Xbox);
        let mut s = PadSettings::default();
        assert!(!s.conflicts("key.jump"));
        s.bind("key.drop", Some(Button::South));
        assert!(s.conflicts("key.jump") && s.conflicts("key.drop"));
        assert!(!s.is_default("key.drop") && s.is_default("key.jump"));
        assert_eq!(action_caption(HOTBAR_NEXT), "Hotbar: Next Slot");
        assert_eq!(action_caption("key.jump"), "key.jump");
    }

    #[test]
    fn hotbar_wraps_and_glyphs_follow_the_vendor() {
        assert_eq!(hotbar_step(8, true), 0);
        assert_eq!(hotbar_step(0, false), 8);
        assert_eq!(hotbar_step(3, true), 4);
        assert_eq!(GlyphStyle::of_vendor(Some(0x054c)), GlyphStyle::PlayStation);
        assert_eq!(GlyphStyle::of_vendor(Some(0x28de)), GlyphStyle::Xbox);
        assert_eq!(GlyphStyle::of_vendor(None), GlyphStyle::Xbox);
    }
}
