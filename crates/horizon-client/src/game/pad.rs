//! Controllers through gilrs (Horizon's own; docs/controllers.md): the
//! active pad's state each loop iteration, its button edges dispatched as
//! the vanilla key mappings they are bound to, the left stick as an analog
//! move vector and the right stick turning the view. `horizon-ctl pad`
//! injects a virtual pad on top of the real one.
use super::{Game, Screen};
use horizon_client::{
    Result,
    gamepad::{self, Button, Edges, GlyphStyle, PadState},
    keybinds::{KeyBinds, key_code, mouse_button},
    mouse::Rotation,
};
use serde_json::{Value, json};
use std::time::Instant;
use winit::keyboard::KeyCode;

pub(super) const HELP: &[&str] = &[
    "pad BUTTON down|up|tap         a virtual controller button (south east west north lb rb lt rt select start ls rs up down left right)",
    "pad stick left|right X Y       tilt a virtual stick (-1..1, up positive); 0 0 centres it",
    "pad trigger left|right VALUE   pull a virtual trigger (0..1)",
    "pad release                    centre and release the virtual controller",
    "pad status                     connected controllers, the active one and its state",
];

/// The controllers and the state the game last acted on.
#[derive(Default)]
pub(super) struct Pads {
    gilrs: Option<gilrs::Gilrs>,
    started: bool,
    /// The pad last touched; only its input counts.
    active: Option<gilrs::GamepadId>,
    /// The virtual pad of `horizon-ctl pad`.
    injected: PadState,
    /// The virtual pad was in use at the last poll.
    virtual_input: bool,
    /// Buttons a `pad ... tap` releases at the next poll.
    taps: Vec<Button>,
    edges: Edges,
    /// The merged state of the last poll.
    state: PadState,
    /// The pad drove the game last (the mouse and keyboard hand control back).
    pub(super) in_use: bool,
    pub(super) style: GlyphStyle,
}

fn gilrs_button(button: Button) -> gilrs::Button {
    use gilrs::Button as G;
    match button {
        Button::South => G::South,
        Button::East => G::East,
        Button::West => G::West,
        Button::North => G::North,
        Button::LeftBumper => G::LeftTrigger,
        Button::RightBumper => G::RightTrigger,
        Button::LeftTrigger => G::LeftTrigger2,
        Button::RightTrigger => G::RightTrigger2,
        Button::Select => G::Select,
        Button::Start => G::Start,
        Button::LeftStick => G::LeftThumb,
        Button::RightStick => G::RightThumb,
        Button::DPadUp => G::DPadUp,
        Button::DPadDown => G::DPadDown,
        Button::DPadLeft => G::DPadLeft,
        Button::DPadRight => G::DPadRight,
    }
}

impl Pads {
    /// Open gilrs on first use. A platform without it (or a Linux without
    /// udev) leaves controllers off; the game runs as before.
    fn start(&mut self) {
        if self.started {
            return;
        }
        self.started = true;
        self.gilrs = match gilrs::Gilrs::new() {
            Ok(g) => {
                for (_, pad) in g.gamepads() {
                    log::info!("Controller connected: {}", pad.name());
                }
                Some(g)
            }
            Err(gilrs::Error::NotImplemented(_)) => None,
            Err(e) => {
                log::warn!("Horizon: controllers are unavailable: {e}");
                None
            }
        };
    }

    /// Drain gilrs' events and read the active pad.
    fn read(&mut self) -> PadState {
        let Some(gilrs) = self.gilrs.as_mut() else {
            return PadState::default();
        };
        while let Some(gilrs::Event { id, event, .. }) = gilrs.next_event() {
            use gilrs::EventType as E;
            match event {
                E::Connected => {
                    log::info!("Controller connected: {}", gilrs.gamepad(id).name());
                }
                E::Disconnected => {
                    log::info!("Controller disconnected: {}", gilrs.gamepad(id).name());
                    if self.active == Some(id) {
                        self.active = None;
                    }
                }
                E::ButtonPressed(..) => self.active = Some(id),
                E::AxisChanged(_, value, _) | E::ButtonChanged(_, value, _)
                    if value.abs() > 0.5 =>
                {
                    self.active = Some(id)
                }
                _ => {}
            }
        }
        let Some(pad) = self.active.and_then(|id| gilrs.connected_gamepad(id)) else {
            return PadState::default();
        };
        self.style = GlyphStyle::of_vendor(pad.vendor_id());
        let mut state = PadState::default();
        for button in Button::ALL {
            if !matches!(button, Button::LeftTrigger | Button::RightTrigger) {
                state.set(button, pad.is_pressed(gilrs_button(button)));
            }
        }
        let axis = |a| pad.value(a);
        use gilrs::Axis as A;
        state.left = [axis(A::LeftStickX), axis(A::LeftStickY)];
        state.right = [axis(A::RightStickX), axis(A::RightStickY)];
        // Triggers arrive as analog buttons on most mappings, as plain
        // buttons on some.
        let trigger = |b: gilrs::Button| {
            let value = pad.button_data(b).map_or(0.0, |d| d.value());
            if pad.is_pressed(b) {
                value.max(1.0)
            } else {
                value
            }
        };
        state.triggers = [
            trigger(gilrs::Button::LeftTrigger2),
            trigger(gilrs::Button::RightTrigger2),
        ];
        state
    }
}

impl Game<'_> {
    /// Once per loop iteration: read the pads and act on button changes.
    pub(super) fn poll_pads(&mut self) {
        let settings = &self.settings.controller;
        if !settings.enabled {
            if self.pad.state != PadState::default() {
                self.pad.state = PadState::default();
                for (button, down) in self.pad.edges.release_all() {
                    self.pad_edge(button, down);
                }
            }
            return;
        }
        self.pad.start();
        let hardware = self.pad.read();
        let state = hardware.merge(&self.pad.injected);
        self.pad.virtual_input = self.pad.injected != PadState::default();
        for button in std::mem::take(&mut self.pad.taps) {
            self.pad.injected.set(button, false);
        }
        let threshold = self.settings.controller.trigger_threshold;
        let edges = self.pad.edges.update(&state, threshold);
        if state.active(&self.settings.controller) {
            self.last_input = Instant::now();
            self.pad.in_use = true;
        }
        self.pad.state = state;
        for (button, down) in edges {
            self.pad_edge(button, down);
        }
    }

    fn pad_edge(&mut self, button: Button, down: bool) {
        if let Err(e) = self.pad_button(button, down) {
            log::warn!("Horizon: controller {}: {e}", button.name());
        }
    }

    /// Input from the pad counts while the window has focus, or always for
    /// the virtual pad (a script can't focus the window).
    fn pad_focused(&self) -> bool {
        self.focused || self.pad.virtual_input
    }

    fn pad_button(&mut self, button: Button, down: bool) -> Result<()> {
        if down && !self.pad_focused() {
            return Ok(());
        }
        let playing = self.title.is_none() && self.screen == Screen::Playing;
        if down && playing && !self.captured && self.focused {
            // Like a click into the window: take the pointer.
            self.capture(true);
        }
        if button == Button::Start {
            return if down { self.pad_back() } else { Ok(()) };
        }
        if !playing && down {
            // Until menus take the pad, it can at least leave them.
            return match button {
                Button::East => self.pad_back(),
                Button::South if self.screen == Screen::Dead && self.title.is_none() => {
                    // The respawn button, as if clicked.
                    let size = self
                        .renderer
                        .as_ref()
                        .map_or([1, 1], |r| [r.config.width, r.config.height]);
                    let b = horizon_client::hud::HudBatch::new(size);
                    let (x, y, _, _) = super::screens::death_buttons(b.width, b.height)[0];
                    let cursor = std::mem::replace(&mut self.cursor, (x + 100.0, y + 10.0));
                    let result = self.death_click();
                    self.cursor = cursor;
                    result
                }
                Button::North if matches!(self.screen, Screen::Inventory | Screen::Creative) => {
                    self.mapped_key(KeyCode::KeyE, true, false)
                }
                _ => Ok(()),
            };
        }
        // Releases always reach the mappings, so nothing stays held when a
        // screen opened in between.
        for action in self.settings.controller.actions_of(button) {
            self.pad_action(action, down)?;
        }
        Ok(())
    }

    /// A bound action: a vanilla mapping acts as its default key or mouse
    /// button, like a rebound keyboard key.
    fn pad_action(&mut self, action: &str, down: bool) -> Result<()> {
        match action {
            gamepad::HOTBAR_PREVIOUS | gamepad::HOTBAR_NEXT => {
                if down && self.screen == Screen::Playing {
                    self.selected =
                        gamepad::hotbar_step(self.selected, action == gamepad::HOTBAR_NEXT);
                }
                Ok(())
            }
            "key.sneak"
                if self.settings.controller.toggle_sneak && !self.settings.toggle_crouch =>
            {
                // A stick click flips sneaking instead of holding it.
                if down {
                    let sneaking = self.keys.contains(&KeyCode::ShiftLeft);
                    self.mapped_key(KeyCode::ShiftLeft, !sneaking, false)?;
                }
                Ok(())
            }
            _ => {
                let default = KeyBinds::default_of(action);
                if let Some(button) = mouse_button(default) {
                    if self.title.is_none() && self.screen == Screen::Playing {
                        self.play_mouse(button, down)?;
                    }
                    Ok(())
                } else if let Some(key) = key_code(default) {
                    self.mapped_key(key, down, false)
                } else {
                    Ok(())
                }
            }
        }
    }

    /// B and Start: what Escape does where the pad is.
    fn pad_back(&mut self) -> Result<()> {
        if let Some(title) = self.title.as_mut() {
            title.escape();
            return Ok(());
        }
        if self.sign_edit.is_some() {
            self.close_sign_editor();
            return Ok(());
        }
        if self.book.is_some() {
            self.close_book();
            return Ok(());
        }
        if let Some(remote) = self.remote.as_mut()
            && remote.chat_input.is_some()
        {
            remote.chat_input = None;
            self.capture(true);
            return Ok(());
        }
        self.key_pressed(KeyCode::Escape)
    }

    /// The left stick as the move vector, while playing.
    pub(super) fn pad_move(&self) -> Option<[f32; 2]> {
        let settings = &self.settings.controller;
        if !settings.enabled
            || self.title.is_some()
            || self.screen != Screen::Playing
            || !self.pad_focused()
        {
            return None;
        }
        gamepad::move_vector(self.pad.state.left, settings.move_deadzone)
    }

    /// The right stick turns the view, once per frame (`Entity.turn`).
    pub(super) fn pad_look(&mut self, seconds: f32) {
        let settings = &self.settings.controller;
        if !settings.enabled
            || self.title.is_some()
            || self.screen != Screen::Playing
            || self.player.is_none()
            || !self.pad_focused()
        {
            return;
        }
        // A long frame (a hitch) must not spin the view.
        let (yaw, pitch) = gamepad::look_delta(
            self.pad.state.right,
            settings,
            seconds.min(0.1),
            self.scoping(),
        );
        if yaw == 0.0 && pitch == 0.0 {
            return;
        }
        let mut rotation = Rotation {
            x_rot: self.pitch,
            y_rot: self.yaw,
            x_rot_o: self.pitch,
            y_rot_o: self.yaw,
        };
        rotation.turn(f64::from(yaw / 0.15), f64::from(pitch / 0.15));
        self.yaw = rotation.y_rot;
        self.pitch = rotation.x_rot;
    }

    /// `horizon-ctl pad ...`.
    pub(super) fn pad_command(&mut self, words: &[&str]) -> Result<Value> {
        let arg = |i: usize| -> Result<&str> {
            words
                .get(i)
                .copied()
                .ok_or_else(|| format!("usage: {}", HELP[0]).into())
        };
        let number = |i: usize| -> Result<f32> {
            let v: f32 = arg(i)?.parse()?;
            if v.is_finite() {
                Ok(v)
            } else {
                Err("not a number".into())
            }
        };
        match arg(1)? {
            "status" => {
                let pads: Vec<Value> = self.pad.gilrs.as_ref().map_or(Vec::new(), |g| {
                    g.gamepads()
                        .map(|(id, p)| {
                            json!({
                                "name": p.name(),
                                "vendor": p.vendor_id(),
                                "product": p.product_id(),
                                "active": self.pad.active == Some(id),
                            })
                        })
                        .collect()
                });
                let s = &self.pad.state;
                let held: Vec<&str> = Button::ALL
                    .into_iter()
                    .filter(|b| self.pad.edges.is_held(*b))
                    .map(Button::name)
                    .collect();
                Ok(json!({
                    "enabled": self.settings.controller.enabled,
                    "backend": self.pad.gilrs.is_some(),
                    "controllers": pads,
                    "glyphs": self.pad.style.name(),
                    "in_use": self.pad.in_use,
                    "held": held,
                    "left": s.left,
                    "right": s.right,
                    "triggers": s.triggers,
                }))
            }
            "release" => {
                self.pad.injected = PadState::default();
                self.pad.taps.clear();
                Ok(json!({ "ok": true }))
            }
            "stick" => {
                let v = [number(3)?.clamp(-1.0, 1.0), number(4)?.clamp(-1.0, 1.0)];
                match arg(2)? {
                    "left" => self.pad.injected.left = v,
                    "right" => self.pad.injected.right = v,
                    other => return Err(format!("stick must be left or right, not {other}").into()),
                }
                Ok(json!({ "ok": true }))
            }
            "trigger" => {
                let v = number(3)?.clamp(0.0, 1.0);
                match arg(2)? {
                    "left" => self.pad.injected.triggers[0] = v,
                    "right" => self.pad.injected.triggers[1] = v,
                    other => {
                        return Err(format!("trigger must be left or right, not {other}").into());
                    }
                }
                Ok(json!({ "ok": true }))
            }
            name => {
                let button = Button::parse(name)
                    .ok_or_else(|| format!("unknown pad button {name}; {}", HELP[0]))?;
                match arg(2)? {
                    "down" => self.pad.injected.set(button, true),
                    "up" => self.pad.injected.set(button, false),
                    "tap" => {
                        self.pad.injected.set(button, true);
                        self.pad.taps.push(button);
                    }
                    other => {
                        return Err(
                            format!("pad action must be down, up or tap, not {other}").into()
                        );
                    }
                }
                Ok(json!({ "ok": true }))
            }
        }
    }
}
