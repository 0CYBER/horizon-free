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
            button(b, a, bounds, self.label(control), pressed, color);
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

    /// The label a control's button shows.
    fn label(&self, control: Control) -> &'static str {
        let vertical = self.swimming || self.vertical_movement;
        match control {
            Control::Move => "",
            Control::Sprint => "Sprint",
            Control::Jump if vertical => "Up",
            Control::Jump => "Jump",
            Control::Sneak if vertical => "Down",
            Control::Sneak if self.mounted => "Dismount",
            Control::Sneak => "Sneak",
            Control::Attack => "Attack",
            Control::Use => "Use",
            Control::Chat => "Chat",
            Control::Pause => "II",
            Control::Settings => "Touch",
            Control::Inventory => "...",
            Control::Debug => "F3",
            Control::Perspective => "F5",
            Control::Offhand => "Swap",
            Control::Shield => "Shield",
        }
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
            for (dx, dy, label, pressed) in [
                (1.0, 0.0, "^", state.left[1] > 0.2),
                (0.0, 1.0, "<", state.left[0] < -0.2),
                (2.0, 1.0, ">", state.left[0] > 0.2),
                (1.0, 2.0, "v", state.left[1] < -0.2),
            ] {
                let bounds = [x + cell * dx, y + cell * dy, cell, cell];
                button(b, a, bounds, label, pressed, color);
            }
            return;
        }
        let middle = [x + size / 2.0, y + size / 2.0];
        let [r, g, blue, opacity] = color.to_le_bytes();
        ring(b, a, middle, size / 2.0, color);
        disc(b, a, middle, size / 2.0, rgba(0, 0, 0, opacity / 3));
        let knob = [
            middle[0] + state.left[0] * size / 4.0,
            middle[1] - state.left[1] * size / 4.0,
        ];
        disc(b, a, knob, size * 0.17, rgba(r, g, blue, opacity / 2));
        ring(b, a, knob, size * 0.17, color);
    }
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
    // Shrink a long label to fit, never grow a short one.
    let scale = ((w - 6.0).max(1.0) / width.max(1.0)).min(((h - 6.0).max(1.0) / 8.0).min(1.0));
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

    #[test]
    fn labels_and_held_buttons_follow_the_player() {
        let mut t = TouchControls::default();
        assert_eq!(t.label(Control::Jump), "Jump");
        t.swimming = true;
        assert_eq!(t.label(Control::Jump), "Up");
        assert_eq!(t.label(Control::Sneak), "Down");
        t.swimming = false;
        t.mounted = true;
        assert_eq!(t.label(Control::Sneak), "Dismount");
        let state = t.state();
        assert!(!t.held(Control::Sneak, &state));
        t.sneaking = true;
        assert!(t.held(Control::Sneak, &state));
        t.update(1, t.placement(Control::Attack).center, true, false);
        assert!(t.held(Control::Attack, &t.state()));
        assert!(!t.held(Control::Use, &t.state()));
    }
}
