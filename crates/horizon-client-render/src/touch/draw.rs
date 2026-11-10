// Adapted from Starling (https://github.com/starling-client/starling, `android` branch at
// 150a414b09a1, `crates/horizon-client-render/src/touch.rs`), written by Vasysik and the
// Starling contributors. Copyright (c) 2026 Ludovic Marie and the Horizon contributors, under the
// same MIT license as Horizon (LICENSE at the repository root).

//! Drawing the touch controls on the HUD, in GUI pixels. Buttons use the
//! resource pack's own button sprite (`widget/button`, highlighted while held)
//! with a short label; the joystick, the D-pad and the tap marker are plain
//! shapes. No art of its own ships with the game.
use super::{Control, Mode, Placement, Role, TouchControls, layout};
use crate::gamepad::{Button, PadState};
use crate::hud::{HudAtlas, HudBatch, rgba};
use crate::lang;

const BUTTON: &str = "gui/sprites/widget/button";
const BUTTON_HELD: &str = "gui/sprites/widget/button_highlighted";

impl TouchControls {
    /// Draws every visible control, then the tap marker in the tap schemes.
    pub fn draw(&self, b: &mut HudBatch, a: &HudAtlas) {
        let state = self.state();
        for control in Control::ALL {
            if !self.visible(control) {
                continue;
            }
            let p = self.placement(control);
            let color = rgba(255, 255, 255, alpha(p.opacity));
            if control == Control::Move {
                self.draw_move(b, a, p, &state, color);
                continue;
            }
            let bounds = square(b, p.center, p.size);
            let rect = p.rect(self.aspect());
            let pressed = self.held(control, &state)
                || self
                    .fingers
                    .values()
                    .any(|(_, point)| layout::contains(rect, *point));
            button(b, a, bounds, &self.label(control), pressed, color);
        }
        if let Some(point) = self.pointer() {
            let center = [point[0] * b.width, point[1] * b.height];
            let radius = self.settings.circle_size / 2.0;
            ring(b, a, center, radius + 0.7, rgba(0, 0, 0, 110));
            ring(b, a, center, radius, rgba(255, 255, 255, 210));
            // Mining fills the circle outwards from its centre.
            let inner = mining_radius(radius, self.break_progress);
            if inner > 0.0 {
                disc(b, a, center, inner, rgba(255, 255, 255, 90));
                ring(b, a, center, inner, rgba(255, 255, 255, 220));
            }
        }
    }

    /// Whether a control shows as held: its button is down, or it latched.
    fn held(&self, control: Control, state: &PadState) -> bool {
        if control == Control::Sneak && self.sneaking {
            return true;
        }
        Self::button(control).is_some_and(|button| match button {
            Button::LeftTrigger => state.triggers[0] > 0.0,
            Button::RightTrigger => state.triggers[1] > 0.0,
            _ => state.held(button),
        })
    }

    /// The label a control's button shows, in the player's language. Key
    /// names and symbols ("F3", "II") read the same in every language.
    fn label(&self, control: Control) -> String {
        let vertical = self.swimming || self.vertical_movement;
        let key = match control {
            Control::Move => return String::new(),
            Control::Pause => return "II".to_owned(),
            Control::Inventory => return "...".to_owned(),
            Control::Debug => return "F3".to_owned(),
            Control::Perspective => return "F5".to_owned(),
            Control::Sprint => "horizon.touch.sprint",
            Control::Jump if vertical => "horizon.touch.up",
            Control::Jump => "horizon.touch.jump",
            Control::Sneak if vertical => "horizon.touch.down",
            Control::Sneak if self.mounted => "horizon.touch.dismount",
            Control::Sneak => "horizon.touch.sneak",
            Control::Attack => "horizon.touch.attack",
            Control::Use => "horizon.touch.use",
            Control::Chat => "horizon.touch.chat",
            Control::Settings => "horizon.touch.settings",
            Control::Offhand => "horizon.touch.swap",
            Control::Shield => "horizon.touch.shield",
        };
        lang::get(key)
    }

    /// The joystick (a rim and a knob) or the D-pad (four arrow buttons).
    fn draw_move(
        &self,
        b: &mut HudBatch,
        a: &HudAtlas,
        p: Placement,
        state: &PadState,
        color: u32,
    ) {
        let anchor = self.fingers.values().find_map(|(role, _)| match role {
            Role::Move(anchor) => Some(*anchor),
            _ => None,
        });
        let center = match anchor {
            Some(anchor) if self.dynamic_joystick() => anchor,
            // A dynamic joystick only shows while a finger holds it.
            None if self.dynamic_joystick() => return,
            _ => p.center,
        };
        let [x, y, size, _] = square(b, center, p.size);
        if self.settings.mode == Mode::Dpad {
            let cell = size / 3.0;
            for ([dx, dy], label, pressed) in dpad(state.left) {
                let bounds = [x + cell * dx, y + cell * dy, cell, cell];
                button(b, a, bounds, label, pressed, color);
            }
            return;
        }
        let middle = [x + size / 2.0, y + size / 2.0];
        let [r, g, blue, opacity] = color.to_le_bytes();
        ring(b, a, middle, size / 2.0, color);
        disc(b, a, middle, size / 2.0, rgba(0, 0, 0, opacity / 3));
        let knob = knob(middle, state.left, size);
        disc(b, a, knob, size * 0.17, rgba(r, g, blue, opacity / 2));
        ring(b, a, knob, size * 0.17, color);
    }
}

/// The D-pad's arrows: each one's cell in a 3x3 grid, its label and whether
/// the stick (`left`, up positive) presses it.
fn dpad(left: [f32; 2]) -> [([f32; 2], &'static str, bool); 4] {
    [
        ([1.0, 0.0], "^", left[1] > 0.2),
        ([0.0, 1.0], "<", left[0] < -0.2),
        ([2.0, 1.0], ">", left[0] > 0.2),
        ([1.0, 2.0], "v", left[1] < -0.2),
    ]
}

/// Where the joystick's knob sits: the stick is up positive, the screen down.
fn knob(middle: [f32; 2], left: [f32; 2], size: f32) -> [f32; 2] {
    [
        middle[0] + left[0] * size / 4.0,
        middle[1] - left[1] * size / 4.0,
    ]
}

/// A placement's square in GUI pixels, `[x, y, size, size]`.
fn square(b: &HudBatch, center: [f32; 2], size: f32) -> [f32; 4] {
    let size = size * b.height;
    [
        center[0] * b.width - size / 2.0,
        center[1] * b.height - size / 2.0,
        size,
        size,
    ]
}

fn alpha(opacity: f32) -> u8 {
    (opacity.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// A labelled button: the pack's button sprite, or a plain frame when the
/// pack has none. A held button darkens and its label drops a pixel.
fn button(
    b: &mut HudBatch,
    a: &HudAtlas,
    bounds: [f32; 4],
    label: &str,
    pressed: bool,
    color: u32,
) {
    let [x, y, w, h] = bounds;
    let [r, g, blue, opacity] = color.to_le_bytes();
    if a.sprites.contains_key(BUTTON) {
        let sprite = if pressed && a.sprites.contains_key(BUTTON_HELD) {
            BUTTON_HELD
        } else {
            BUTTON
        };
        crate::ui::nine_slice(b, a, sprite, x, y, w, h, color);
    } else {
        let fill = if pressed { 150 } else { 90 };
        b.fill(
            a,
            x,
            y,
            w,
            h,
            rgba(0, 0, 0, (u16::from(opacity) * fill / 255) as u8),
        );
        for edge in [
            [x, y, w, 1.0],
            [x, y + h - 1.0, w, 1.0],
            [x, y, 1.0, h],
            [x + w - 1.0, y, 1.0, h],
        ] {
            b.fill(a, edge[0], edge[1], edge[2], edge[3], color);
        }
    }
    if label.is_empty() {
        return;
    }
    let text = if pressed {
        rgba(r / 2 + 127, g / 2 + 127, blue / 2, opacity)
    } else {
        color
    };
    let drop = if pressed { (h / 18.0).min(2.0) } else { 0.0 };
    let width = HudBatch::text_width(a, label);
    let start = b.vertices.len();
    b.text(
        a,
        label,
        x + (w - width) / 2.0,
        y + (h - 8.0) / 2.0 + drop,
        text,
        true,
    );
    // Shrink a long label to fit, never grow a short one: the width leaves 3
    // pixels each side, and the height cap keeps the 8-pixel line inside the
    // same margin, never above full size.
    let fit_width = (w - 6.0).max(1.0) / width.max(1.0);
    let height_cap = ((h - 6.0).max(1.0) / 8.0).min(1.0);
    let scale = fit_width.min(height_cap);
    b.scale_since(start, x + w / 2.0, y + h / 2.0 + drop, scale, scale);
}

/// How far the mining disc reaches inside a marker of `radius`, by progress.
fn mining_radius(radius: f32, progress: f32) -> f32 {
    if progress.is_finite() {
        (radius - 1.5).max(0.0) * progress.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// 65 points round the unit circle, the last one equal to the first.
fn circle_points() -> &'static [[f32; 2]; 65] {
    static POINTS: std::sync::OnceLock<[[f32; 2]; 65]> = std::sync::OnceLock::new();
    POINTS.get_or_init(|| {
        std::array::from_fn(|i| {
            let angle = i as f32 / 64.0 * std::f32::consts::TAU;
            [angle.cos(), angle.sin()]
        })
    })
}

/// A 1.4-pixel circle line, one quad per segment so nothing blends twice.
fn ring(b: &mut HudBatch, a: &HudAtlas, center: [f32; 2], radius: f32, color: u32) {
    let at = |p: [f32; 2], r: f32| [center[0] + p[0] * r, center[1] + p[1] * r];
    let (outer, inner) = (radius + 0.7, (radius - 0.7).max(0.0));
    for s in circle_points().windows(2) {
        b.fill_quad(
            a,
            [
                at(s[0], outer),
                at(s[1], outer),
                at(s[1], inner),
                at(s[0], inner),
            ],
            color,
        );
    }
}

/// A filled circle.
fn disc(b: &mut HudBatch, a: &HudAtlas, center: [f32; 2], radius: f32, color: u32) {
    let at = |p: [f32; 2]| [center[0] + p[0] * radius, center[1] + p[1] * radius];
    for s in circle_points().windows(2) {
        b.fill_quad(a, [center, at(s[0]), at(s[1]), center], color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn atlas() -> HudAtlas {
        HudAtlas::build(std::path::Path::new("/nonexistent/horizon-touch-draw")).unwrap()
    }

    fn vertices(t: &TouchControls, a: &HudAtlas) -> usize {
        let mut b = HudBatch::new([1920, 1080]);
        t.draw(&mut b, a);
        b.vertices.len()
    }

    #[test]
    fn mining_disc_grows_outwards_within_the_marker() {
        assert_eq!(mining_radius(16.0, 0.0), 0.0);
        assert_eq!(mining_radius(16.0, 0.5), 7.25);
        assert_eq!(mining_radius(16.0, 1.0), 14.5);
        assert_eq!(mining_radius(16.0, 2.0), 14.5);
        assert_eq!(mining_radius(16.0, -1.0), 0.0);
        assert_eq!(mining_radius(16.0, f32::NAN), 0.0);
    }

    #[test]
    fn hidden_controls_draw_nothing() {
        let a = atlas();
        let mut t = TouchControls::default();
        assert!(vertices(&t, &a) > 0);
        t.settings.button_visibility = [layout::HIDDEN; layout::CONTROLS];
        assert_eq!(vertices(&t, &a), 0);
    }

    #[test]
    fn a_dynamic_joystick_shows_only_while_held() {
        let a = atlas();
        let mut t = TouchControls::default();
        t.settings.dynamic_joystick = true;
        let idle = vertices(&t, &a);
        t.update(1, [0.1, 0.6], true, false);
        assert!(vertices(&t, &a) > idle);
        t.update(1, [0.1, 0.6], false, true);
        assert_eq!(vertices(&t, &a), idle);
    }

    #[test]
    fn the_marker_shows_in_tap_schemes_and_fills_while_mining() {
        let a = atlas();
        let mut t = TouchControls::default();
        let idle = vertices(&t, &a);
        t.update(1, [0.6, 0.4], true, false);
        assert_eq!(vertices(&t, &a), idle, "no marker with a crosshair");
        t.clear();
        t.settings.mode = Mode::Tap;
        let idle = vertices(&t, &a);
        t.update(1, [0.6, 0.4], true, false);
        let marker = vertices(&t, &a);
        assert!(marker > idle);
        t.break_progress = 0.5;
        assert!(vertices(&t, &a) > marker);
    }

    fn english<R>(f: impl FnOnce() -> R) -> R {
        let (result, missed) = lang::scoped(lang::Language::load(&[], "en_us"), f);
        assert!(missed.is_empty(), "no English text for {missed:?}");
        result
    }

    #[test]
    fn labels_and_held_buttons_follow_the_player() {
        let mut t = TouchControls::default();
        let all = |t: &TouchControls| english(|| Control::ALL.map(|c| t.label(c)));
        assert_eq!(all(&t)[..3], ["", "Sprint", "Jump"]);
        assert!(all(&t).iter().all(|l| !l.starts_with("horizon.")));
        assert_eq!(t.label(Control::Debug), "F3");
        t.swimming = true;
        assert_eq!(english(|| t.label(Control::Jump)), "Up");
        assert_eq!(english(|| t.label(Control::Sneak)), "Down");
        t.swimming = false;
        t.mounted = true;
        assert_eq!(english(|| t.label(Control::Sneak)), "Dismount");
        let state = t.state();
        assert!(!t.held(Control::Sneak, &state));
        t.sneaking = true;
        assert!(t.held(Control::Sneak, &state));
        t.update(1, t.placement(Control::Attack).center, true, false);
        assert!(t.held(Control::Attack, &t.state()));
        assert!(!t.held(Control::Use, &t.state()));
    }

    #[test]
    fn colours_split_into_rgba_in_order() {
        let half = rgba(255, 255, 255, alpha(0.5));
        assert_eq!(half.to_le_bytes(), [255, 255, 255, 128]);
        assert_eq!(rgba(10, 20, 30, 40).to_le_bytes(), [10, 20, 30, 40]);
    }

    /// One button drawn alone, as `button` draws it in `draw`.
    fn one(a: &HudAtlas, bounds: [f32; 4], label: &str, pressed: bool) -> HudBatch {
        let mut b = HudBatch::new([1920, 1080]);
        button(&mut b, a, bounds, label, pressed, rgba(255, 255, 255, 128));
        b
    }

    #[test]
    fn a_held_button_uses_the_packs_highlighted_sprite() {
        let mut a = atlas();
        let rect = |u0: f32| crate::hud::Rect {
            u0,
            v0: 0.0,
            u1: u0 + 0.1,
            v1: 0.1,
            w: 200,
            h: 20,
        };
        a.sprites.insert(BUTTON.to_owned(), rect(0.2));
        a.sprites.insert(BUTTON_HELD.to_owned(), rect(0.5));
        let uses = |b: &HudBatch, u0: f32| {
            b.vertices
                .iter()
                .any(|v| (u0..=u0 + 0.1).contains(&v.uv[0]) && v.uv[1] <= 0.1)
        };
        let idle = one(&a, [0.0, 0.0, 60.0, 20.0], "", false);
        assert!(uses(&idle, 0.2) && !uses(&idle, 0.5));
        let held = one(&a, [0.0, 0.0, 60.0, 20.0], "", true);
        assert!(uses(&held, 0.5) && !uses(&held, 0.2));
        assert!(
            held.vertices
                .iter()
                .all(|v| v.color == rgba(255, 255, 255, 128))
        );
    }

    #[test]
    fn without_the_sprite_a_plain_frame_darkens_while_held() {
        let a = atlas();
        assert!(!a.sprites.contains_key(BUTTON));
        let colours = |pressed| {
            let b = one(&a, [10.0, 10.0, 60.0, 20.0], "", pressed);
            b.vertices.iter().map(|v| v.color).collect::<Vec<_>>()
        };
        let (idle, held) = (colours(false), colours(true));
        let mut fill = HudBatch::new([1920, 1080]);
        fill.fill(&a, 0.0, 0.0, 1.0, 1.0, 0);
        let quad = fill.vertices.len();
        // The backdrop, then the four one-pixel edges in the button's colour.
        assert_eq!(idle.len(), 5 * quad);
        assert!(idle[..quad].iter().all(|&c| c == rgba(0, 0, 0, 45)));
        assert!(held[..quad].iter().all(|&c| c == rgba(0, 0, 0, 75)));
        assert!(idle[quad..].iter().all(|&c| c == rgba(255, 255, 255, 128)));
    }

    #[test]
    fn a_long_label_shrinks_to_fit_and_a_short_one_keeps_its_size() {
        let a = atlas();
        let frame = one(&a, [0.0, 0.0, 40.0, 20.0], "", false).vertices.len();
        let span = |label: &str| {
            let b = one(&a, [0.0, 0.0, 40.0, 20.0], label, false);
            assert!(b.vertices.len() > frame, "{label} drew no text");
            let xs = b.vertices[frame..].iter().map(|v| v.pos[0]);
            let (lo, hi) = xs.fold((f32::MAX, f32::MIN), |(lo, hi), x| (lo.min(x), hi.max(x)));
            (lo, hi)
        };
        let long = "a label much wider than its button";
        assert!(HudBatch::text_width(&a, long) > 40.0);
        let (lo, hi) = span(long);
        assert!(lo >= 3.0 - 1e-3 && hi <= 37.0 + 1e-3, "{lo}..{hi}");
        let width = HudBatch::text_width(&a, "Up");
        let (lo, hi) = span("Up");
        assert!(hi - lo >= width - 1e-3, "{} < {width}", hi - lo);
    }

    #[test]
    fn the_dpad_and_knob_follow_the_stick_up_positive() {
        let only = |left: [f32; 2]| {
            let on: Vec<_> = dpad(left).into_iter().filter(|p| p.2).collect();
            assert_eq!(on.len(), 1, "{left:?}");
            (on[0].0, on[0].1)
        };
        assert_eq!(only([0.0, 0.5]), ([1.0, 0.0], "^"));
        assert_eq!(only([0.0, -0.5]), ([1.0, 2.0], "v"));
        assert_eq!(only([-0.5, 0.0]), ([0.0, 1.0], "<"));
        assert_eq!(only([0.5, 0.0]), ([2.0, 1.0], ">"));
        assert!(dpad([0.1, -0.1]).iter().all(|p| !p.2));
        // Stick up draws the knob above the middle, on screen.
        assert_eq!(knob([50.0, 50.0], [0.0, 1.0], 40.0), [50.0, 40.0]);
        assert_eq!(knob([50.0, 50.0], [1.0, 0.0], 40.0), [60.0, 50.0]);
    }
}
