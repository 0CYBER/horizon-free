//! The keyboard drives a menu's focus: the arrow keys, Tab and Enter step and press the
//! targets the screen registered, with the same focus the controller uses (a pure move out of
//! `pad_menu.rs`, the keyboard-focus half; docs/controllers.md).
use super::super::event_loop::EventLoopCtl;
use super::super::{
    Game, Screen,
    key_focus::{self, Act},
};
use horizon_client::{
    gamepad::Rect,
    screen_narrator::InputType,
    widgets::{self, Kind, Target},
};
use winit::{
    event::{DeviceId, ElementState, MouseButton, WindowEvent},
    keyboard::KeyCode,
};

impl Game<'_> {
    /// The keyboard drives the open menu's focus.
    pub(in crate::game) fn key_focus_driving(&self) -> bool {
        self.pad.menu.keyboard && self.key_focus_screen()
    }

    /// A pointer event the keyboard sends (a press, the hidden pointer on
    /// the focused widget) is being handled: the screen's narration doesn't
    /// take it for the mouse (`InputType` stays the keyboard's).
    pub(in crate::game) fn key_focus_quiet(&self) -> bool {
        self.pad.menu.synthetic && self.pad.menu.keyboard
    }

    /// The screen keeps the navigation keys for itself: typing in chat (Tab
    /// completes a command), a sign, a book, the creative, recipe-book and
    /// mods searches, the anvil's name, a command block, a dialog, a key or
    /// button being bound, the credits, the on-screen keyboard.
    fn key_focus_blocked(&self) -> bool {
        let playing_screen = |screen| self.title.is_none() && self.screen == screen;
        self.pad_keyboard_captures()
            || playing_screen(Screen::Chat)
            || self.sign_edit.is_some()
            || matches!(
                self.book,
                Some(super::super::books::BookScreen::Edit { .. })
            )
            || (playing_screen(Screen::Creative) && self.creative_screen.tab().search)
            || self.recipe_search_focused()
            || self.mod_browser_typing()
            || self.station_typing()
            || self.screen == Screen::CommandBlock
            || self.screen == Screen::Win
            || self.dialog_open()
            || (self.screen == Screen::Options
                && (self.options_waiting_for_key()
                    || self.ui.options.selected_pad_action.is_some()))
    }

    /// `Screen.clearFocus`, which `Gui.setScreen` runs on the screen it takes
    /// down: the keyboard's focus belongs to that screen. A key bound to the
    /// container's own close key can move the focus and close the screen in
    /// one press (Tab bound to the inventory), and vanilla clears the focus
    /// before the next frame, so nothing stays highlighted.
    pub(in crate::game) fn pad_clear_focus(&mut self) {
        self.pad.menu.clear_focus();
    }

    /// A key pressed in a menu, before the screen takes it
    /// (`Screen.keyPressed`): Tab, Shift+Tab and the arrows move the focus,
    /// Enter, Space and keypad Enter press the focused widget; the focused
    /// widget's own keys come first (a text field's caret, a slider's
    /// value). Whether the key was used; when not, the screen has it (a
    /// container also has Tab and the arrows once they moved the focus).
    pub(in crate::game) fn key_focus_key(
        &mut self,
        event_loop: &dyn EventLoopCtl,
        key: KeyCode,
    ) -> bool {
        let Some(nav) = key_focus::nav_of(key, self.has_shift_down()) else {
            return false;
        };
        if !self.key_focus_screen() || self.key_focus_blocked() {
            return false;
        }
        let targets = self.key_targets();
        if targets.is_empty() {
            return false;
        }
        let find = |rect: Rect| {
            targets
                .iter()
                .find(|t| t.rect.iter().zip(rect).all(|(a, b)| (a - b).abs() < 0.5))
                .copied()
        };
        // The focus: the keyboard's or the controller's, else what a click
        // focused (a slider a click focused changes with the arrows).
        let mut slider_edit = self.pad.menu.slider_edit;
        let mut focus = self
            .pad
            .menu
            .focus
            .filter(|_| self.pad_focus_mode())
            .and_then(|f| find(f.rect));
        if focus.is_none()
            && let Some(clicked) = self.pad.menu.origin.and_then(find)
        {
            focus = Some(clicked);
            slider_edit = true;
        }
        let ctx = key_focus::Context {
            slider_edit,
            typing: self.edit_box_focused(),
            rows_take_enter: self.title.as_ref().is_some_and(|t| t.list_focused()),
            gui: self.gui_size(),
        };
        let act = key_focus::act(nav, focus, &targets, ctx);
        if act == Act::Pass {
            return false;
        }
        if !self.pad.menu.keyboard {
            self.pad.menu.key_screen = self.key_screen();
            self.pad.menu.key_restart = None;
        }
        let menu = &mut self.pad.menu;
        menu.keyboard = true;
        menu.origin = None;
        menu.focus = focus;
        menu.slider_edit = slider_edit;
        if menu.pos.is_none() {
            menu.pos = Some(self.cursor);
        }
        match (act, focus) {
            (Act::Focus(i), _) => {
                self.pad_focus_on(event_loop, targets[i]);
                self.pad.menu.slider_edit = key_focus::slider_edit_on_focus(nav);
            }
            (Act::Step(button), Some(focus)) => {
                if !self.pad_step_value(event_loop, button, focus) {
                    self.pad_focus_move(event_loop, button, focus, &targets);
                }
            }
            (Act::Press, Some(focus)) => {
                let focus = self.pad_reveal(focus);
                self.pad.menu.focus = Some(focus);
                // AbstractButton.keyPressed: onPress, with its click sound.
                self.pad_click_at(event_loop, focus.center(), false);
            }
            (Act::ToggleSlider, _) => self.pad.menu.slider_edit = !slider_edit,
            (Act::Nudge(sign), Some(focus)) => self.pad_nudge(event_loop, focus, sign),
            _ => {}
        }
        // The narration reads a focused slider's `canChangeValue` from the
        // input type (Tab: it can, an arrow: it can't); keep them in step
        // after Enter flipped it or a click let it.
        if let Some(Target {
            kind: Kind::Slider { .. },
            ..
        }) = self.pad.menu.focus
        {
            self.narration.set_input(if self.pad.menu.slider_edit {
                InputType::KeyboardTab
            } else {
                InputType::KeyboardArrow
            });
        }
        let container =
            self.title.is_none() && matches!(self.screen, Screen::Inventory | Screen::Creative);
        !key_focus::passes_on(act, container)
    }

    /// Which screen is open, as far as the keyboard's focus cares.
    pub(in crate::game) fn key_screen(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        std::mem::discriminant(&self.screen).hash(&mut hasher);
        self.remote_menu_id().hash(&mut hasher);
        if self.screen == Screen::Options {
            std::mem::discriminant(&self.ui.options.page()).hash(&mut hasher);
        }
        self.title
            .as_ref()
            .map(|t| t.screen_key())
            .hash(&mut hasher);
        (self.sign_edit.is_some(), self.book.is_some()).hash(&mut hasher);
        self.pad_prompt_key().hash(&mut hasher);
        hasher.finish()
    }

    /// Each poll while the keyboard has the focus: another screen opened
    /// (a pressed button opened it), so once it is drawn the focus starts on
    /// its first widget (`Screen.setInitialFocus` after a key). Whether the
    /// focus is waiting for that or just started again.
    pub(super) fn key_focus_restart(
        &mut self,
        event_loop: &dyn EventLoopCtl,
        targets: &[Target],
    ) -> bool {
        if !self.pad.menu.keyboard {
            return false;
        }
        let screen = self.key_screen();
        if screen != self.pad.menu.key_screen {
            self.pad.menu.key_screen = screen;
            self.pad.menu.key_restart = Some(widgets::generation());
            self.pad.menu.pending = None;
        }
        let Some(changed_at) = self.pad.menu.key_restart else {
            return false;
        };
        // Two frames on: the first may have been drawn before the click.
        if widgets::generation() > changed_at + 1 {
            self.pad.menu.key_restart = None;
            self.pad.menu.focus = None;
            if let Some(first) = self.pad_first_target(targets) {
                self.pad_focus_on(event_loop, first);
            }
        }
        true
    }

    /// The keyboard's focus moved from `left` to `target`
    /// (`ComponentPath.applyFocus`): a slider's arrows don't change it yet,
    /// a text field takes the typing (a click at its end, where Tab leaves
    /// the caret) and the one left behind lets go of it, and a world row is
    /// selected (`AbstractSelectionList.setFocused`).
    pub(super) fn key_focus_landed(
        &mut self,
        event_loop: &dyn EventLoopCtl,
        left: Option<Target>,
        target: Target,
    ) {
        self.pad.menu.slider_edit = false;
        if left.is_some_and(|l| l.rect == target.rect) {
            return;
        }
        if left.is_some_and(|l| l.kind == Kind::Field)
            && target.kind != Kind::Field
            && let Some(title) = self.title.as_mut()
        {
            title.blur_field();
        }
        match target.kind {
            Kind::Field => {
                self.pad_click_at(event_loop, key_focus::field_click(target.rect), false);
            }
            Kind::Row => {
                let size = self.gui_size();
                if let Some(title) = self.title.as_mut() {
                    title.focus_world_row(target.center(), size);
                }
            }
            _ => {}
        }
    }

    /// The real mouse, before the screen takes it: where it is, and a press
    /// or a scroll hands it the pointer back from the keyboard, at the
    /// place the mouse left it (`setLastInputType(MOUSE)`).
    pub(in crate::game) fn key_focus_mouse(
        &mut self,
        event_loop: &dyn EventLoopCtl,
        event: &WindowEvent,
    ) {
        if self.pad.menu.synthetic {
            return;
        }
        let takeover = match event {
            // Not while the pad holds the radial menu open: as for raw mouse motion in play
            // (`device_event`), a cursor that moved is a bumped mouse.
            WindowEvent::CursorMoved { position, .. } => {
                self.pad.menu.real_pointer != Some(*position) && !self.pad_radial_captures()
            }
            WindowEvent::KeyboardInput { event, .. } => event.state == ElementState::Pressed,
            WindowEvent::MouseInput { state, .. } => *state == ElementState::Pressed,
            WindowEvent::MouseWheel { .. } => true,
            _ => false,
        };
        if takeover {
            self.pad_yield_input(event_loop);
        }
        match event {
            WindowEvent::CursorMoved { position, .. } => {
                self.pad.menu.real_pointer = Some(*position);
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                ..
            } => self.key_focus_to_mouse(event_loop),
            WindowEvent::MouseWheel { .. } => self.key_focus_to_mouse(event_loop),
            _ => {}
        }
    }

    /// Only a press accepted by ScreenEvent.Pre may focus the widget under it.
    pub(in crate::game) fn key_focus_mouse_press(&mut self, event: &WindowEvent) {
        if !self.pad.menu.synthetic
            && self.key_focus_screen()
            && matches!(
                event,
                WindowEvent::MouseInput {
                    state: ElementState::Pressed,
                    button: MouseButton::Left,
                    ..
                }
            )
        {
            let (cx, cy) = self.cursor;
            self.pad.menu.origin = self
                .key_targets()
                .into_iter()
                .find(|t| {
                    let [x, y, w, h] = t.rect;
                    cx >= x && cy >= y && cx < x + w && cy < y + h
                })
                .map(|t| t.rect);
        }
    }

    /// The keyboard lets go of the pointer: it goes back to the real mouse.
    fn key_focus_to_mouse(&mut self, event_loop: &dyn EventLoopCtl) {
        if !std::mem::take(&mut self.pad.menu.keyboard) {
            return;
        }
        let menu = &mut self.pad.menu;
        menu.origin = menu.focus.take().map(|t| t.rect);
        menu.pos = None;
        menu.pending = None;
        menu.nudge = None;
        let real = menu.real_pointer;
        if let (Some(position), Some(id)) = (real, self.window.as_ref().map(|w| w.id())) {
            self.pad_synthetic_event(
                event_loop,
                id,
                WindowEvent::CursorMoved {
                    device_id: DeviceId::dummy(),
                    position,
                },
            );
        }
    }

    /// Each poll: what a click focused is forgotten once it is gone (the
    /// screen changed), and out of menus.
    pub(super) fn key_focus_frame(&mut self) {
        let Some(origin) = self.pad.menu.origin else {
            return;
        };
        let there = self.key_focus_screen()
            && self
                .key_targets()
                .iter()
                .any(|t| t.rect.iter().zip(origin).all(|(a, b)| (a - b).abs() < 0.5));
        if !there {
            self.pad.menu.origin = None;
        }
    }

    /// The keyboard focus the title had while resources loaded
    /// (`StartupMenu`), carried into the game's.
    pub(in crate::game) fn key_focus_adopt(&mut self, focus: Target, slider_edit: bool) {
        let screen = self.key_screen();
        let menu = &mut self.pad.menu;
        menu.key_screen = screen;
        menu.key_restart = None;
        menu.keyboard = true;
        menu.focus = Some(focus);
        menu.slider_edit = slider_edit;
        menu.pos = Some(self.cursor);
    }
}
