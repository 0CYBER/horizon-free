//! Controllers through gilrs (Horizon's own; docs/controllers.md): the
//! active pad's state each loop iteration, its button edges dispatched as
//! the vanilla key mappings they are bound to, the left stick as an analog
//! move vector and the right stick turning the view. `horizon-ctl pad`
//! injects a virtual pad on top of the real one.
use super::{Game, Screen};
use horizon_client::{
    Result,
    gamepad::{self, Button, Edges, GlyphStyle, HealthWatch, PadState, Rumble, TriggerFeel},
    keybinds::{KeyBinds, key_code, mouse_button},
    mouse::Rotation,
};
use serde_json::{Value, json};
use std::time::{Duration, Instant};
use winit::{event_loop::ActiveEventLoop, keyboard::KeyCode};

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
    /// The game tick each button went down in, and releases held back until
    /// a tick has passed: vanilla samples keys once per tick, so a click
    /// shorter than that (a quick stick click for sprint) would be lost.
    pressed_at: [Option<u64>; 16],
    late_releases: Vec<Button>,
    /// The merged state of the last poll.
    pub(super) state: PadState,
    /// The pad drove the game last (the mouse and keyboard hand control back).
    pub(super) in_use: bool,
    pub(super) style: GlyphStyle,
    /// The cursor the pad drives in menus.
    pub(super) menu: super::pad_menu::MenuCursor,
    /// Vibrations playing (an effect stops when dropped) and when they end.
    rumbles: Vec<(gilrs::ff::Effect, Instant)>,
    /// The last vibration asked for, for `pad status`.
    last_rumble: Option<(&'static str, gamepad::Pulse)>,
    /// The player's health between polls (a drop is damage taken).
    health: HealthWatch,
    /// The active pad is a DualSense.
    dualsense: bool,
    /// Its own output over HID (rumble here; gilrs only reads it), started once this window
    /// has the pad in hand; dropped, it releases the pad.
    output: Option<horizon_dualsense::Link>,
    /// Its HD haptics, on USB: looked for once each time the output opens.
    haptics: Option<horizon_dualsense::haptics::Haptics>,
    haptics_tried: bool,
    /// What it was last told to show.
    shown: Option<horizon_dualsense::device::State>,
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

fn is_dualsense(pad: &gilrs::Gamepad) -> bool {
    matches!((pad.vendor_id(), pad.product_id()),
        (Some(vendor), Some(product)) if horizon_dualsense::is_dualsense(vendor, product))
}

impl Pads {
    /// A new player entity took over (spawn, respawn, a server joined, a
    /// dimension change): its health is not compared with the old one's.
    pub(super) fn player_replaced(&mut self) {
        self.health.forget();
    }
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
                    if let Some(output) = &self.output {
                        output.scan();
                    }
                }
                E::Disconnected => {
                    log::info!("Controller disconnected: {}", gilrs.gamepad(id).name());
                    if self.active == Some(id) {
                        self.active = None;
                    }
                    if let Some(output) = &self.output {
                        output.scan();
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
            // Before any pad is touched (the virtual pad playing), a DualSense still shows it.
            self.dualsense = gilrs.gamepads().any(|(_, pad)| is_dualsense(&pad));
            return PadState::default();
        };
        self.style = GlyphStyle::of_vendor(pad.vendor_id());
        self.dualsense = is_dualsense(&pad);
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
    pub(super) fn poll_pads(&mut self, event_loop: &ActiveEventLoop) {
        let settings = &self.settings.controller;
        if !settings.enabled {
            // Dropping the output frees the triggers.
            self.pad.output = None;
            self.pad.shown = None;
            if self.pad.state != PadState::default() {
                self.pad.state = PadState::default();
                for (button, down) in self.pad.edges.release_all() {
                    self.pad_edge(event_loop, button, down);
                }
            }
            return;
        }
        self.pad.start();
        let hardware = self.pad.read();
        self.ui.options.pad_style = self.settings.controller.glyphs.resolve(self.pad.style);
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
        if self.pad.dualsense && self.pad.in_use && self.focused && self.pad.output.is_none() {
            self.pad.output = Some(horizon_dualsense::Link::start());
        }
        self.pad_haptics();
        self.pad_show();
        // Damage from any source, here or on a server; nothing counts on
        // the title screen.
        let health = self
            .player
            .as_ref()
            .filter(|_| self.title.is_none())
            .map(|p| p.health.health());
        if let Some(damage) = self.pad.health.observe(health) {
            self.pad_rumble(Rumble::Hurt(damage));
        }
        let now = Instant::now();
        self.pad.rumbles.retain(|(_, end)| *end > now);
        let ticking = self.title.is_none() && self.screen == Screen::Playing && !self.pause;
        for button in std::mem::take(&mut self.pad.late_releases) {
            if !ticking || self.pad.pressed_at[button as usize] != Some(self.ticks) {
                self.pad_button_logged(event_loop, button, false);
            } else {
                self.pad.late_releases.push(button);
            }
        }
        for (button, down) in edges {
            self.pad_edge(event_loop, button, down);
        }
        self.pad_menu_frame(event_loop);
    }

    fn pad_edge(&mut self, event_loop: &ActiveEventLoop, button: Button, down: bool) {
        let ticking = self.title.is_none() && self.screen == Screen::Playing && !self.pause;
        if down {
            // Pressed again before its held-back release went out.
            if let Some(i) = self.pad.late_releases.iter().position(|b| *b == button) {
                self.pad.late_releases.remove(i);
                self.pad_button_logged(event_loop, button, false);
            }
            self.pad.pressed_at[button as usize] = Some(self.ticks);
        } else if ticking && self.pad.pressed_at[button as usize] == Some(self.ticks) {
            self.pad.late_releases.push(button);
            return;
        }
        self.pad_button_logged(event_loop, button, down);
    }

    fn pad_button_logged(&mut self, event_loop: &ActiveEventLoop, button: Button, down: bool) {
        if let Err(e) = self.pad_button(event_loop, button, down) {
            log::warn!("Horizon: controller {}: {e}", button.name());
        }
    }

    /// Input from the pad counts while the window has focus, or always for
    /// the virtual pad (a script can't focus the window).
    fn pad_focused(&self) -> bool {
        self.focused || self.pad.virtual_input
    }

    fn pad_button(
        &mut self,
        event_loop: &ActiveEventLoop,
        button: Button,
        down: bool,
    ) -> Result<()> {
        if down && !self.pad_focused() {
            return Ok(());
        }
        let playing = !self.pad_in_menu();
        if down && playing && !self.captured && self.focused {
            // Like a click into the window: take the pointer.
            self.capture(true);
        }
        if !playing {
            self.pad_menu_button(event_loop, button, down)?;
            if down {
                return Ok(());
            }
        } else if button == Button::Start {
            return if down { self.pad_back() } else { Ok(()) };
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
    pub(super) fn pad_back(&mut self) -> Result<()> {
        // The options open over the title screen too: they come first, as
        // for the keyboard's Escape.
        if self.screen == Screen::Options {
            return self.options_key(KeyCode::Escape, None);
        }
        if let Some(title) = self.title.as_mut() {
            title.escape();
            self.save_title_options();
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
        self.key_pressed(KeyCode::Escape)
    }

    /// The left stick as the move vector, while playing.
    pub(super) fn pad_move(&self) -> Option<[f32; 2]> {
        let settings = &self.settings.controller;
        if !settings.enabled || self.pad_in_menu() || !self.pad_focused() {
            return None;
        }
        gamepad::move_vector(self.pad.state.left, settings.move_deadzone)
    }

    /// The right stick turns the view, once per frame (`Entity.turn`).
    pub(super) fn pad_look(&mut self, seconds: f32) {
        let settings = &self.settings.controller;
        if !settings.enabled || self.pad_in_menu() || self.player.is_none() || !self.pad_focused() {
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

    /// The DualSense's HID output, while it is the active pad, open, and this window has focus
    /// (another game on the same pad keeps it otherwise).
    fn pad_output(&self) -> Option<&horizon_dualsense::Link> {
        let output = self.pad.output.as_ref()?;
        (self.pad.dualsense && self.focused && output.open().is_some()).then_some(output)
    }

    /// What the DualSense shows: while playing, its triggers feel what their actions do; in
    /// menus, paused, unfocused or with another pad active, they are free.
    fn pad_show(&mut self) {
        if self.pad.output.is_none() {
            return;
        }
        let playing = self.pad.dualsense
            && self.focused
            && self.title.is_none()
            && self.screen == Screen::Playing
            && !self.pause;
        let (using, attacking) = if playing {
            self.trigger_feels()
        } else {
            (TriggerFeel::Free, TriggerFeel::Free)
        };
        // The lightbar follows the player; menus keep the last colour.
        let in_world = self.title.is_none();
        let lightbar = match self.player.as_mut().filter(|_| in_world) {
            Some(p) => {
                let max = p.attribute("minecraft:max_health").unwrap_or(20.0) as f32;
                let health = p.health.health() / max.max(1.0);
                self.settings
                    .controller
                    .lightbar
                    .colour(health, self.dim.id())
            }
            None => self.pad.shown.and_then(|s| s.lightbar),
        };
        let lights = self.settings.controller.lightbar != gamepad::Lightbar::Off;
        let state = horizon_dualsense::device::State {
            triggers: self.settings.controller.trigger_effects(using, attacking),
            lightbar: lightbar.filter(|_| lights),
            player_leds: lights.then_some(horizon_dualsense::PLAYER_1),
        };
        if self.pad.shown != Some(state)
            && let Some(output) = &self.pad.output
        {
            output.set(state);
            self.pad.shown = Some(state);
        }
    }

    /// What `key.use` and `key.attack` are doing, for the triggers bound to them.
    fn trigger_feels(&self) -> (TriggerFeel, TriggerFeel) {
        let using = match self.using {
            Some((slot, ticks)) => match self.stack_in(slot) {
                Some(s) if &*s.item == "minecraft:bow" => {
                    TriggerFeel::Draw(gamepad::bow_power(ticks))
                }
                // TridentItem.THROW_THRESHOLD_TIME.
                Some(s) if &*s.item == "minecraft:trident" => {
                    TriggerFeel::Draw(ticks as f32 / 10.0)
                }
                Some(s) if &*s.item == "minecraft:crossbow" => {
                    TriggerFeel::Draw(ticks as f32 / Self::charge_ticks(s).max(1) as f32)
                }
                Some(s) if self.blocks_attacks(s).is_some() => TriggerFeel::Brace,
                _ => TriggerFeel::Free,
            },
            None => self.idle_use_feel(),
        };
        // Creative breaks at once; blocks that can't be broken have a negative hardness.
        let attacking = match &self.mining {
            Some(m) if !self.creative => {
                let state = self.world.block(m.pos[0], m.pos[1], m.pos[2]);
                TriggerFeel::Texture(self.res.facts.get(state).hardness)
            }
            _ => TriggerFeel::Free,
        };
        (using, attacking)
    }

    /// The use trigger while nothing is in use: a loaded crossbow clicks, an item that boosts (a
    /// rocket while gliding, the stick of the mount being ridden) catches, a shield weighs, and
    /// locks while an axe has disabled it.
    fn idle_use_feel(&self) -> TriggerFeel {
        let gliding = self.player.as_ref().is_some_and(|p| p.fall_flying);
        let mount = self
            .riding
            .and_then(|id| self.mobs.iter().find(|m| m.id == id));
        let mount = mount.map_or("", |m| m.kind.id.as_str());
        let feel = |stack: Option<&horizon_client::items::ItemStack>| match stack
            .map(|s| (s, &*s.item))
        {
            Some((s, "minecraft:crossbow")) if s.data().is_some_and(|d| !d.charged.is_empty()) => {
                TriggerFeel::Click
            }
            Some((_, "minecraft:firework_rocket")) if gliding => TriggerFeel::Boost,
            Some((_, "minecraft:carrot_on_a_stick")) if mount == "minecraft:pig" => {
                TriggerFeel::Boost
            }
            Some((_, "minecraft:warped_fungus_on_a_stick")) if mount == "minecraft:strider" => {
                TriggerFeel::Boost
            }
            Some((s, _)) if self.blocks_attacks(s).is_some() => match self.shield_cooldown {
                0 => TriggerFeel::Brace,
                _ => TriggerFeel::Locked,
            },
            _ => TriggerFeel::Free,
        };
        // Use tries the main hand first.
        match feel(self.stack_in(self.selected)) {
            TriggerFeel::Free => feel(self.offhand.as_ref()),
            main => main,
        }
    }

    /// Open the pad's haptics once its output is open over USB (its audio device is there
    /// only then); close them when it goes. Opt-in with `HORIZON_DUALSENSE_HAPTICS=1` until
    /// they are proven on a pad (docs/controllers.md); never for games without sound (tests).
    fn pad_haptics(&mut self) {
        let usb = self.pad.output.as_ref().and_then(|o| o.open())
            == Some(horizon_dualsense::Transport::Usb);
        if !usb {
            self.pad.haptics = None;
            self.pad.haptics_tried = false;
        } else if !self.pad.haptics_tried {
            self.pad.haptics_tried = true;
            if std::env::var_os("HORIZON_DUALSENSE_HAPTICS").is_some_and(|v| v == "1")
                && std::env::var_os("HORIZON_NO_AUDIO").is_none()
            {
                self.pad.haptics = horizon_dualsense::haptics::Haptics::open();
            }
        }
    }

    /// Release the DualSense's output (the game is quitting).
    pub(super) fn pad_close_output(&mut self) {
        self.pad.haptics = None;
        self.pad.output = None;
    }

    /// Vibrate the active pad, when vibration is on and the pad is the one
    /// playing (a pad lying idle while the mouse plays stays still): a
    /// DualSense through its own report, else gilrs' force feedback, which
    /// platforms without it (gilrs on macOS) skip.
    pub(super) fn pad_rumble(&mut self, rumble: Rumble) {
        let settings = &self.settings.controller;
        if !settings.enabled || !settings.vibration || !self.pad.in_use {
            return;
        }
        let Some(pulse) = rumble.pulse() else {
            return;
        };
        self.pad.last_rumble = Some((rumble.name(), pulse));
        let level = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        if let Some(haptics) = self
            .pad
            .haptics
            .as_ref()
            .filter(|_| self.pad_output().is_some())
        {
            let steps = rumble.shape().into_iter().map(|step| {
                (
                    step.strong,
                    step.weak,
                    Duration::from_millis(u64::from(step.millis)),
                )
            });
            haptics.play(&steps.collect::<Vec<_>>());
            return;
        }
        if let Some(output) = self.pad_output() {
            let steps = rumble.shape().into_iter().map(|step| {
                let length = Duration::from_millis(u64::from(step.millis));
                ([level(step.strong), level(step.weak)], length)
            });
            output.rumble(steps.collect());
            return;
        }
        let (Some(gilrs), Some(id)) = (self.pad.gilrs.as_mut(), self.pad.active) else {
            return;
        };
        if !gilrs
            .connected_gamepad(id)
            .is_some_and(|g| g.is_ff_supported())
        {
            return;
        }
        use gilrs::ff::{BaseEffect, BaseEffectType, EffectBuilder, Repeat, Replay, Ticks};
        let length = Ticks::from_ms(pulse.millis);
        let motor = |kind| BaseEffect {
            kind,
            scheduling: Replay {
                play_for: length,
                ..Default::default()
            },
            envelope: Default::default(),
        };
        let magnitude = |v: f32| (v.clamp(0.0, 1.0) * f32::from(u16::MAX)) as u16;
        let effect = EffectBuilder::new()
            .add_effect(motor(BaseEffectType::Strong {
                magnitude: magnitude(pulse.strong),
            }))
            .add_effect(motor(BaseEffectType::Weak {
                magnitude: magnitude(pulse.weak),
            }))
            .repeat(Repeat::For(length))
            .gamepads(&[id])
            .finish(gilrs);
        match effect.and_then(|e| e.play().map(|()| e)) {
            Ok(effect) => {
                let end = Instant::now() + Duration::from_millis(u64::from(pulse.millis) + 50);
                self.pad.rumbles.push((effect, end));
            }
            Err(e) => log::debug!("Horizon: controller vibration: {e}"),
        }
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
                    "dualsense_output": self.pad.output.as_ref().and_then(|o| o.open())
                        .map(|t| format!("{t:?}")),
                    "trigger_effects": self.pad.shown.map(|s| s.triggers),
                    "last_rumble": self.pad.last_rumble.map(|(why, p)| json!({
                        "reason": why, "strong": p.strong, "weak": p.weak, "ms": p.millis,
                    })),
                    "in_use": self.pad.in_use,
                    "menu": self.pad_menu_status(),
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
