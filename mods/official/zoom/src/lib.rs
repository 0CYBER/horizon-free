//! Horizon's official zoom, formerly the built-in extra: hold the key (Z
//! by default) to zoom in, and turn the mouse wheel while holding it to zoom
//! further in or out. Its settings page has the extra's four options: the
//! game lists the key there first and keeps the player's choice
//! (`horizon-mods.txt`); the rest is kept in the mod's storage (`level`,
//! `smooth`, `sensitivity`),
//! where the game writes a player's old extra settings when it installs the
//! mod for them.
use horizon_mod_sdk::{
    Mod, Registration,
    input::{self, Event},
    render::{self, Zoom as ZoomView},
    storage,
    ui::{self, ButtonWidget, Widget},
};

/// The key's name for `register-key`.
const KEY: &str = "zoom";
/// The zooms the starting zoom cycles through, and the wheel's limits.
const LEVELS: [u8; 6] = [2, 3, 4, 6, 8, 10];
const MIN: f32 = 1.25;
const MAX: f32 = 10.0;
/// Each wheel step multiplies the zoom by this.
const STEP: f32 = 1.25;
/// Settings page buttons, under the key's row.
const LEVEL_BUTTON: u32 = 2;
const SMOOTH_BUTTON: u32 = 3;
const SENSITIVITY_BUTTON: u32 = 4;

struct Zoom {
    /// The zoom the key starts at, one of `LEVELS`.
    level: u8,
    /// Ease in and out like the spyglass.
    smooth: bool,
    /// Turn slower the more you zoom in.
    sensitivity: bool,
    /// The zoom while the key is held; None when it is up.
    held: Option<f32>,
}

impl Zoom {
    /// Tells the game the zoom in effect.
    fn show(&self) {
        let factor = self.held.unwrap_or(1.0);
        render::set_zoom(ZoomView {
            factor,
            smooth: self.smooth,
            turn: if self.sensitivity { 1.0 / factor } else { 1.0 },
        });
    }
    fn page(&self) {
        let on = |b: bool| if b { "ON" } else { "OFF" };
        ui::set_settings(&[
            Widget::Button(ButtonWidget {
                id: LEVEL_BUTTON,
                label: format!("Starting zoom: {}x", self.level),
            }),
            Widget::Button(ButtonWidget {
                id: SMOOTH_BUTTON,
                label: format!("Smooth zoom: {}", on(self.smooth)),
            }),
            Widget::Button(ButtonWidget {
                id: SENSITIVITY_BUTTON,
                label: format!("Slower turning: {}", on(self.sensitivity)),
            }),
        ]);
    }
    fn save(&self) {
        let flag = |b: bool| if b { &b"true"[..] } else { &b"false"[..] };
        let _ = storage::set("level", self.level.to_string().as_bytes());
        let _ = storage::set("smooth", flag(self.smooth));
        let _ = storage::set("sensitivity", flag(self.sensitivity));
    }
}

fn stored(key: &str) -> Option<String> {
    storage::get(key).and_then(|v| String::from_utf8(v).ok())
}

impl Mod for Zoom {
    fn init() -> Result<(Self, Registration), String> {
        input::register_key(KEY, "Zoom key", "key.keyboard.z")?;
        // The extra's defaults: 4x, smooth, slower turning.
        let level = stored("level")
            .and_then(|v| v.trim().parse::<u8>().ok())
            .map_or(4, |v| v.clamp(2, MAX as u8));
        let flag = |key: &str| stored(key).is_none_or(|v| v.trim() != "false");
        let zoom = Zoom {
            level,
            smooth: flag("smooth"),
            sensitivity: flag("sensitivity"),
            held: None,
        };
        zoom.page();
        Ok((zoom, Registration::new().on_frame()))
    }
    fn frame(&mut self, _seconds: f32) {
        let mut changed = false;
        for event in input::poll() {
            match event {
                Event::KeyDown(k) if k == KEY => {
                    self.held = Some(f32::from(self.level).clamp(MIN, MAX));
                    // While held, the wheel zooms instead of moving the hotbar.
                    input::claim_wheel(true);
                }
                Event::KeyUp(k) if k == KEY => {
                    self.held = None;
                    input::claim_wheel(false);
                }
                Event::Wheel(steps) => {
                    let Some(z) = &mut self.held else { continue };
                    *z = (*z * STEP.powf(steps)).clamp(MIN, MAX);
                }
                _ => continue,
            }
            changed = true;
        }
        if changed {
            self.show();
        }
    }
    fn ui_select(&mut self, id: u32, _row: u32) {
        match id {
            LEVEL_BUTTON => {
                // As the extra cycled: an unknown level counts as the first.
                let i = LEVELS.iter().position(|&l| l == self.level).unwrap_or(0);
                self.level = LEVELS[(i + 1) % LEVELS.len()];
            }
            SMOOTH_BUTTON => self.smooth = !self.smooth,
            SENSITIVITY_BUTTON => self.sensitivity = !self.sensitivity,
            _ => return,
        }
        self.save();
        self.page();
    }
    fn ui_settings_opened(&mut self) {
        self.page();
    }
}

horizon_mod_sdk::export_mod!(Zoom);
