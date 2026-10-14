//! Horizon's extras in the game (`docs/extras.md`): the zoom key and wheel,
//! clicks for the keystroke overlay, frame times for the FPS counter, and
//! their HUD. Each hook does nothing while its extra is off.
use super::{Game, Screen};
use horizon_client::{
    extras::{Cps, FpsCounter, Zoom, fov_target, turn_scale},
    extras_hud::{self, Corners, Keystrokes},
    hud::HudBatch,
    keybinds,
};
use std::time::Duration;
use winit::{event::MouseButton, keyboard::KeyCode};

#[derive(Default)]
pub(super) struct ExtrasState {
    zoom: Zoom,
    fps: FpsCounter,
    /// Left and right mouse buttons: held, and their clicks.
    mouse: [bool; 2],
    cps: [Cps; 2],
}

impl Game<'_> {
    fn extras_now(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }
    fn zoom_key(&self) -> Option<KeyCode> {
        let e = &self.settings.extras;
        e.zoom.then(|| keybinds::key_code(&e.zoom_key)).flatten()
    }
    /// A key press or release while playing; true when the zoom key took it.
    pub(super) fn extras_key(&mut self, key: KeyCode, pressed: bool, repeat: bool) -> bool {
        if self.zoom_key() != Some(key) {
            return false;
        }
        if !pressed {
            self.extras.zoom.release();
        } else if !repeat && self.screen == Screen::Playing && self.title.is_none() {
            self.extras.zoom.press(self.settings.extras.zoom_level);
        }
        true
    }
    /// Wheel steps while playing; true when zooming took them from the hotbar.
    pub(super) fn extras_scroll(&mut self, steps: f32) -> bool {
        self.settings.extras.zoom && self.extras.zoom.scroll(steps)
    }
    /// Mouse buttons while playing, for the keystroke overlay.
    pub(super) fn extras_mouse(&mut self, button: MouseButton, pressed: bool) {
        let i = match button {
            MouseButton::Left => 0,
            MouseButton::Right => 1,
            _ => return,
        };
        self.extras.mouse[i] = pressed;
        if pressed && self.settings.extras.keystrokes {
            let now = self.extras_now();
            self.extras.cps[i].click(now);
        }
    }
    /// Once per frame: the frame's time, and keys let go behind screens.
    pub(super) fn extras_frame(&mut self, elapsed: Duration) {
        // Under agent control the window may lack focus without pausing.
        let focused = self.focused || self.control.is_some();
        let playing = self.screen == Screen::Playing && self.title.is_none() && focused;
        if !playing || !self.settings.extras.zoom {
            self.extras.zoom.release();
        }
        if !playing {
            self.extras.mouse = [false; 2];
        }
        if self.settings.extras.fps {
            let now = self.extras_now();
            self.extras.fps.record(now, elapsed.as_secs_f32() * 1000.0);
        } else {
            self.extras.fps.clear();
        }
    }
    /// The zoom in effect (1 unless the key is held).
    fn zoom_factor(&self) -> f32 {
        self.extras.zoom.factor(self.scoping())
    }
    /// The field-of-view modifier target, divided by a smooth zoom.
    pub(super) fn zoomed_fov_target(&self, vanilla: f32) -> f32 {
        if self.settings.extras.zoom_smooth {
            fov_target(vanilla, self.zoom_factor())
        } else {
            vanilla
        }
    }
    /// The field of view in degrees, divided by an instant zoom.
    pub(super) fn zoomed_fov_degrees(&self, fov: f32) -> f32 {
        if self.settings.extras.zoom_smooth {
            fov
        } else {
            fov / self.zoom_factor()
        }
    }
    /// Mouse turning while zoomed.
    pub(super) fn zoom_turn_scale(&self) -> f64 {
        turn_scale(self.zoom_factor(), self.settings.extras.zoom_sensitivity)
    }
    /// The FPS counter and keystrokes, in the HUD layer (hidden with F1 by
    /// the caller). The top left corner gives way to the debug screen.
    pub(super) fn draw_extras(&mut self, batch: &mut HudBatch) {
        let e = &self.settings.extras;
        if !e.fps && !e.keystrokes {
            return;
        }
        let (fps_on, fps_corner, lows) = (e.fps, e.fps_corner, e.fps_lows);
        let (keys_on, keys_corner, cps_on) = (e.keystrokes, e.keystrokes_corner, e.keystrokes_cps);
        let visible = |corner| !(self.debug && corner == horizon_client::extras::Corner::TopLeft);
        let mut corners = Corners::new(batch.width, batch.height, self.effect_rows_height());
        let atlas = &self.res.atlas;
        if fps_on && visible(fps_corner) {
            let reading = self.extras.fps.reading();
            extras_hud::draw_fps(batch, atlas, &mut corners, fps_corner, reading, lows);
        }
        if keys_on && visible(keys_corner) {
            let now = self.extras_now();
            let cps = cps_on.then(|| {
                let [l, r] = &mut self.extras.cps;
                (l.per_second(now), r.per_second(now))
            });
            let held = |k| self.keys.contains(&k);
            let label = |name| keybinds::display_name(self.settings.keys.get(name)).plain();
            let k = Keystrokes {
                moves: [
                    (held(KeyCode::KeyW), label("key.forward")),
                    (held(KeyCode::KeyA), label("key.left")),
                    (held(KeyCode::KeyS), label("key.back")),
                    (held(KeyCode::KeyD), label("key.right")),
                ],
                jump: held(KeyCode::Space),
                attack: self.extras.mouse[0],
                use_: self.extras.mouse[1],
                cps,
            };
            extras_hud::draw_keystrokes(batch, &self.res.atlas, &mut corners, keys_corner, &k);
        }
    }
    /// The height the status effect icons take at the top right
    /// (`Hud.extractEffects`: beneficial at y 1, harmful at y 26).
    fn effect_rows_height(&self) -> f32 {
        let effects = &self.potions.effects.list;
        if effects.is_empty() || self.screen != Screen::Playing {
            return 0.0;
        }
        let harmful = effects.iter().any(|e| {
            horizon_client::brewing::effect(e.id)
                .is_some_and(|d| d.category != horizon_client::brewing::Category::Beneficial)
        });
        if harmful { 50.0 } else { 25.0 }
    }
}
