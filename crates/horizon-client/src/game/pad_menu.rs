//! A controller in menus, screens and inventories (Horizon's own;
//! docs/controllers.md): a virtual cursor the left stick moves and the
//! D-pad steps between slots and buttons, with A, X and the stick feeding
//! the screens the same pointer events a mouse sends. Every screen keeps
//! its mouse behaviour (drags, sliders, scrollbars) with no pad code of its
//! own.
use super::{Game, Screen};
use horizon_client::{
    Result,
    creative::Creative,
    gamepad::{self, Button},
    hud::{HudBatch, rgba},
    widgets,
};
use std::time::Instant;
use winit::{
    application::ApplicationHandler,
    dpi::PhysicalPosition,
    event::{DeviceId, ElementState, MouseButton, MouseScrollDelta, TouchPhase, WindowEvent},
    event_loop::ActiveEventLoop,
    keyboard::KeyCode,
};

/// D-pad auto-repeat: the first step at once, the next after a pause.
const REPEAT_DELAY: f32 = 0.35;
const REPEAT_EVERY: f32 = 0.09;
/// Lines per second the right stick scrolls at full tilt.
const SCROLL_SPEED: f32 = 12.0;

#[derive(Default)]
pub(super) struct MenuCursor {
    /// Where the pad put the pointer (GUI pixels); `None` while the mouse
    /// has it.
    pos: Option<(f32, f32)>,
    last_poll: Option<Instant>,
    /// The stick moved the cursor last poll (it settles when let go).
    moving: bool,
    wheel: [f32; 2],
    /// The held D-pad direction and when it last stepped.
    repeat: Option<(Button, Instant, bool)>,
    /// Pointer buttons the pad holds down.
    held: [bool; 2],
    /// The OS pointer is hidden while the pad's cursor is drawn.
    os_hidden: bool,
    /// Polls since a menu opened (its widgets are drawn after one frame).
    open_polls: u32,
}

impl Game<'_> {
    /// A menu, a screen or the title is open: the pad drives a cursor.
    pub(super) fn pad_in_menu(&self) -> bool {
        self.title.is_some() || self.screen != Screen::Playing
    }

    fn gui_scale(&self) -> Option<HudBatch> {
        self.renderer
            .as_ref()
            .map(|r| HudBatch::new([r.config.width, r.config.height]))
    }

    /// Where focus can land: buttons drawn last frame, and the slots and
    /// tabs of inventories (centres, GUI pixels).
    fn pad_targets(&self) -> Vec<(f32, f32)> {
        let mut targets: Vec<(f32, f32)> = widgets::last()
            .iter()
            .filter(|w| w.enabled)
            .map(|w| (w.x + w.w / 2.0, w.y + w.h / 2.0))
            .chain(
                widgets::last_targets()
                    .into_iter()
                    .map(|[x, y, w, h]| (x + w / 2.0, y + h / 2.0)),
            )
            .collect();
        if self.title.is_some() {
            return targets;
        }
        let Some(batch) = self.gui_scale() else {
            return targets;
        };
        match self.screen {
            Screen::Inventory => {
                let (ox, oy) = self.inventory_origin(&batch);
                targets.extend(self.slots().into_iter().map(|i| {
                    let (x, y) = self.slot_position(i);
                    (ox + x + 8.0, oy + y + 8.0)
                }));
            }
            Screen::Creative => {
                let (ox, oy) = Self::creative_origin(&batch);
                for r in 0..5 {
                    for c in 0..9 {
                        targets.push((ox + 17.0 + c as f32 * 18.0, oy + 26.0 + r as f32 * 18.0));
                    }
                }
                for c in 0..9 {
                    targets.push((ox + 17.0 + c as f32 * 18.0, oy + 120.0));
                }
                for t in &self.creative_screen.tabs {
                    let (x, y) = Creative::tab_position(t);
                    targets.push((ox + x + 13.0, oy + y + 16.0));
                }
            }
            _ => {}
        }
        targets
    }

    /// Where focus starts when a menu opens: the held hotbar slot in an
    /// inventory, else the first button.
    fn pad_first_target(&self) -> Option<(f32, f32)> {
        let batch = self.gui_scale()?;
        match self.screen {
            Screen::Inventory if self.title.is_none() => {
                let (ox, oy) = self.inventory_origin(&batch);
                let (x, y) = self.slot_position(self.selected);
                return Some((ox + x + 8.0, oy + y + 8.0));
            }
            Screen::Creative if self.title.is_none() => {
                let (ox, oy) = Self::creative_origin(&batch);
                return Some((ox + 17.0 + self.selected as f32 * 18.0, oy + 120.0));
            }
            _ => {}
        }
        widgets::last()
            .iter()
            .find(|w| w.enabled)
            .map(|w| (w.x + w.w / 2.0, w.y + w.h / 2.0))
    }

    /// Send the window a pointer move to `at` (GUI pixels), as the mouse would.
    fn pad_pointer_to(&mut self, event_loop: &ActiveEventLoop, at: (f32, f32)) {
        let (Some(window), Some(batch)) = (self.window.as_ref(), self.gui_scale()) else {
            return;
        };
        let id = window.id();
        let at = (
            at.0.clamp(0.0, batch.width - 1.0),
            at.1.clamp(0.0, batch.height - 1.0),
        );
        let position =
            PhysicalPosition::new(f64::from(at.0 * batch.scale), f64::from(at.1 * batch.scale));
        self.window_event(
            event_loop,
            id,
            WindowEvent::CursorMoved {
                device_id: DeviceId::dummy(),
                position,
            },
        );
        // Read back what the window made of it, so a real mouse move shows.
        self.pad.menu.pos = Some(self.cursor);
    }

    fn pad_pointer_button(&mut self, event_loop: &ActiveEventLoop, right: bool, pressed: bool) {
        let Some(id) = self.window.as_ref().map(|w| w.id()) else {
            return;
        };
        self.pad.menu.held[usize::from(right)] = pressed;
        let button = if right {
            MouseButton::Right
        } else {
            MouseButton::Left
        };
        let state = if pressed {
            ElementState::Pressed
        } else {
            ElementState::Released
        };
        self.window_event(
            event_loop,
            id,
            WindowEvent::MouseInput {
                device_id: DeviceId::dummy(),
                state,
                button,
            },
        );
    }

    fn pad_wheel(&mut self, event_loop: &ActiveEventLoop, x: f32, y: f32) {
        let Some(id) = self.window.as_ref().map(|w| w.id()) else {
            return;
        };
        self.window_event(
            event_loop,
            id,
            WindowEvent::MouseWheel {
                device_id: DeviceId::dummy(),
                delta: MouseScrollDelta::LineDelta(x, y),
                phase: TouchPhase::Moved,
            },
        );
    }

    /// Put the cursor where the pad last left it, or on the first target.
    fn pad_take_pointer(&mut self, event_loop: &ActiveEventLoop) {
        if self.pad.menu.pos.is_some() {
            return;
        }
        let at = self.pad_first_target().unwrap_or(self.cursor);
        self.pad_pointer_to(event_loop, at);
    }

    /// A button in a menu.
    pub(super) fn pad_menu_button(
        &mut self,
        event_loop: &ActiveEventLoop,
        button: Button,
        down: bool,
    ) -> Result<()> {
        if !down {
            match button {
                Button::South if self.pad.menu.held[0] => {
                    self.pad_pointer_button(event_loop, false, false)
                }
                Button::West if self.pad.menu.held[1] => {
                    self.pad_pointer_button(event_loop, true, false)
                }
                b if self.pad.menu.repeat.is_some_and(|r| r.0 == b) => self.pad.menu.repeat = None,
                _ => {}
            }
            return Ok(());
        }
        match button {
            Button::East | Button::Start => return self.pad_back(),
            Button::South | Button::West => {
                self.pad_take_pointer(event_loop);
                self.pad_pointer_button(event_loop, button == Button::West, true);
            }
            // Y in an inventory: a shift-click (quick move).
            Button::North if matches!(self.screen, Screen::Inventory | Screen::Creative) => {
                self.pad_take_pointer(event_loop);
                let held = (
                    self.keys.contains(&KeyCode::ShiftLeft),
                    self.physical_keys.contains(&KeyCode::ShiftLeft),
                );
                self.keys.insert(KeyCode::ShiftLeft);
                self.physical_keys.insert(KeyCode::ShiftLeft);
                self.pad_pointer_button(event_loop, false, true);
                self.pad_pointer_button(event_loop, false, false);
                if !held.0 {
                    self.keys.remove(&KeyCode::ShiftLeft);
                }
                if !held.1 {
                    self.physical_keys.remove(&KeyCode::ShiftLeft);
                }
            }
            Button::DPadUp | Button::DPadDown | Button::DPadLeft | Button::DPadRight => {
                self.pad_focus_step(event_loop, button);
                self.pad.menu.repeat = Some((button, Instant::now(), false));
            }
            // Bumpers switch creative tabs.
            Button::LeftBumper | Button::RightBumper if self.screen == Screen::Creative => {
                self.pad_creative_tab(event_loop, button == Button::RightBumper);
            }
            _ => {}
        }
        Ok(())
    }

    fn pad_focus_step(&mut self, event_loop: &ActiveEventLoop, button: Button) {
        if self.pad.menu.pos.is_none() {
            self.pad_take_pointer(event_loop);
            return;
        }
        // With A held (a slider grabbed, a drag), the D-pad nudges instead.
        if self.pad.menu.held[0] {
            let (dx, dy) = match button {
                Button::DPadLeft => (-2.0, 0.0),
                Button::DPadRight => (2.0, 0.0),
                Button::DPadUp => (0.0, -2.0),
                _ => (0.0, 2.0),
            };
            let at = self.cursor;
            self.pad_pointer_to(event_loop, (at.0 + dx, at.1 + dy));
            return;
        }
        let from = self.cursor;
        let targets = self.pad_targets();
        if let Some(i) = gamepad::focus_step(from, button, &targets) {
            self.pad_pointer_to(event_loop, targets[i]);
        }
    }

    /// Click the next or previous creative tab (wrapping), as a mouse would.
    fn pad_creative_tab(&mut self, event_loop: &ActiveEventLoop, next: bool) {
        let Some(batch) = self.gui_scale() else {
            return;
        };
        let count = self.creative_screen.tabs.len();
        if count == 0 {
            return;
        }
        let selected = self.creative_screen.selected;
        let tab = if next {
            (selected + 1) % count
        } else {
            (selected + count - 1) % count
        };
        let (ox, oy) = Self::creative_origin(&batch);
        let (x, y) = Creative::tab_position(&self.creative_screen.tabs[tab]);
        let back = self.pad.menu.pos;
        self.pad_pointer_to(event_loop, (ox + x + 13.0, oy + y + 16.0));
        self.pad_pointer_button(event_loop, false, true);
        self.pad_pointer_button(event_loop, false, false);
        if let Some(at) = back {
            self.pad_pointer_to(event_loop, at);
        }
    }

    /// Once per poll in a menu: the stick moves the cursor, the right stick
    /// scrolls, a held D-pad repeats, and the OS pointer hides while the
    /// pad's cursor shows.
    pub(super) fn pad_menu_frame(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        let seconds = self
            .pad
            .menu
            .last_poll
            .map_or(0.0, |t| now.duration_since(t).as_secs_f32().min(0.05));
        self.pad.menu.last_poll = Some(now);
        let in_menu = self.pad_in_menu() && self.settings.controller.enabled;
        if !in_menu {
            self.pad.menu.open_polls = 0;
            self.pad.menu.pos = None;
            self.pad.menu.repeat = None;
            self.pad.menu.held = [false; 2];
            self.pad_show_os_pointer(true);
            return;
        }
        self.pad.menu.open_polls = self.pad.menu.open_polls.saturating_add(1);
        // A real mouse move hands the pointer back.
        if let Some(at) = self.pad.menu.pos
            && ((at.0 - self.cursor.0).abs() > 0.5 || (at.1 - self.cursor.1).abs() > 0.5)
        {
            self.pad.menu.pos = None;
            self.pad.in_use = false;
        }
        // Focus lands on the menu as it opens when the pad was in use.
        if self.pad.in_use && self.pad.menu.pos.is_none() && self.pad.menu.open_polls == 3 {
            self.pad_take_pointer(event_loop);
        }
        let (move_deadzone, look_deadzone) = (
            self.settings.controller.move_deadzone,
            self.settings.controller.look_deadzone,
        );
        let state = self.pad.state;
        let (dx, dy) = gamepad::cursor_delta(state.left, move_deadzone, seconds);
        if dx != 0.0 || dy != 0.0 {
            self.pad_take_pointer(event_loop);
            let at = self.cursor;
            self.pad_pointer_to(event_loop, (at.0 + dx, at.1 + dy));
            self.pad.menu.moving = true;
        } else if self.pad.menu.moving {
            // Let go: settle onto a slot or button close by.
            self.pad.menu.moving = false;
            let targets = self.pad_targets();
            if let Some(i) = gamepad::nearest(self.cursor, &targets, 10.0) {
                self.pad_pointer_to(event_loop, targets[i]);
            }
        }
        let scroll = gamepad::radial(state.right, look_deadzone);
        let wheel = &mut self.pad.menu.wheel;
        wheel[0] += scroll[0] * SCROLL_SPEED * seconds;
        wheel[1] += scroll[1] * SCROLL_SPEED * seconds;
        if scroll == [0.0, 0.0] {
            *wheel = [0.0, 0.0];
        }
        let lines = [wheel[0].trunc(), wheel[1].trunc()];
        if lines != [0.0, 0.0] {
            wheel[0] -= lines[0];
            wheel[1] -= lines[1];
            // Stick right scrolls right: the wheel's x runs the other way.
            self.pad_wheel(event_loop, -lines[0], lines[1]);
        }
        if let Some((button, since, repeating)) = self.pad.menu.repeat {
            let wait = if repeating {
                REPEAT_EVERY
            } else {
                REPEAT_DELAY
            };
            if now.duration_since(since).as_secs_f32() >= wait {
                self.pad.menu.repeat = Some((button, now, true));
                self.pad_focus_step(event_loop, button);
            }
        }
        self.pad_show_os_pointer(self.pad.menu.pos.is_none());
    }

    fn pad_show_os_pointer(&mut self, show: bool) {
        if self.pad.menu.os_hidden != show {
            return;
        }
        self.pad.menu.os_hidden = !show;
        if let Some(window) = &self.window {
            window.set_cursor_visible(show && !self.captured);
        }
    }

    /// The pad's cursor, over everything else.
    pub(super) fn draw_pad_cursor(&self, batch: &mut HudBatch) {
        let Some((x, y)) = self.pad.menu.pos else {
            return;
        };
        if !self.pad_in_menu() {
            return;
        }
        // An arrow, outlined, pointing up-left at (x, y).
        let atlas = &self.res.atlas;
        let (x, y) = (x.floor(), y.floor());
        const ROWS: [u8; 10] = [1, 2, 3, 4, 5, 6, 7, 4, 2, 1];
        for (i, &w) in ROWS.iter().enumerate() {
            let row = y + i as f32;
            batch.fill(
                atlas,
                x - 1.0,
                row,
                f32::from(w) + 2.0,
                1.0,
                rgba(0, 0, 0, 255),
            );
        }
        batch.fill(atlas, x - 1.0, y + 10.0, 2.0, 1.0, rgba(0, 0, 0, 255));
        for (i, &w) in ROWS.iter().enumerate().skip(1).take(8) {
            let row = y + i as f32;
            batch.fill(
                atlas,
                x,
                row,
                f32::from(w) - 1.0,
                1.0,
                rgba(255, 255, 255, 255),
            );
        }
    }
}
