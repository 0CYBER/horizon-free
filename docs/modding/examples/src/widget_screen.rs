//! A mod's own screen with two text fields and two buttons (`mod_api::client::widgets`): F8 opens it,
//! Tab walks the widgets, typing edits a field, Copy puts the first field's text in the second,
//! and Done (or Escape) closes it. The same source builds as a native mod and as a mod.wasm.

// ANCHOR: widget_screen
use mod_api::client::widgets::{Button, Outcome, TextField, WidgetId, Widgets};
use mod_api::{
    ClientHost, ClientTickPreEvent, Flow, Hooks, InputKeyEvent, KeyAction, ScreenCharTypedPreEvent,
    ScreenKeyPressedPreEvent, ScreenMouseButtonPressedPreEvent, ScreenMouseButtonReleasedPreEvent,
    ScreenMouseDraggedPreEvent, ScreenRenderPostEvent, Text,
};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

/// The mod's id, which names its screen.
const ID: &str = "widget_example";

/// The screen's widgets and what each one is.
struct Screen {
    widgets: Widgets,
    from: WidgetId,
    to: WidgetId,
    copy: WidgetId,
    done: WidgetId,
}

impl Screen {
    /// The widgets, centred on a screen `width` GUI pixels wide.
    fn new(width: f32) -> Self {
        let x = (width / 2.0 - 100.0).floor();
        let mut widgets = Widgets::new();
        let from = widgets.add_text_field(TextField::new([x, 40.0, 200.0, 20.0], 64));
        let to = widgets.add_text_field(TextField::new([x, 70.0, 200.0, 20.0], 64));
        let copy = widgets.add_button(Button::new([x, 100.0, 98.0, 20.0], Text::literal("Copy")));
        let done = widgets.add_button(Button::new(
            [x + 102.0, 100.0, 98.0, 20.0],
            Text::literal("Done"),
        ));
        Self {
            widgets,
            from,
            to,
            copy,
            done,
        }
    }
}

/// The mod: its screen while it is open.
#[derive(Default)]
pub struct WidgetScreen {
    screen: Mutex<Option<Screen>>,
    /// Done was pressed: the next client tick closes the screen (the input hooks' host only
    /// reads).
    close: AtomicBool,
}

impl WidgetScreen {
    /// Runs `input` on the open screen's widgets and acts on their answer. `Flow::Cancel` when a
    /// widget took the input, so the game does nothing more with it.
    fn input(&self, host: &dyn ClientHost, input: impl FnOnce(&mut Widgets) -> Outcome) -> Flow {
        if host.mod_screen() != Some(ID) {
            return Flow::Continue;
        }
        let mut screen = self.screen.lock().unwrap();
        let Some(screen) = screen.as_mut() else {
            return Flow::Continue;
        };
        let outcome = input(&mut screen.widgets);
        match outcome {
            Outcome::Pressed(id) if id == screen.copy => {
                let text = screen
                    .widgets
                    .text_field(screen.from)
                    .map(|f| f.value().to_owned());
                if let (Some(text), Some(to)) = (text, screen.widgets.text_field_mut(screen.to)) {
                    to.set_value(&text);
                }
            }
            Outcome::Pressed(id) if id == screen.done => self.close.store(true, Ordering::Relaxed),
            _ => {}
        }
        if outcome.handled() {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
}

#[mod_api::horizon_mod]
impl Hooks for WidgetScreen {
    fn input_key(&self, host: &mut dyn ClientHost, event: &mut InputKeyEvent<'_>) {
        if event.action == KeyAction::Press && event.key == "key.keyboard.f8" && host.playing() {
            *self.screen.lock().unwrap() = None;
            host.mod_screen_open(ID);
        }
    }

    fn client_tick_pre(&self, host: &mut dyn ClientHost, _: &mut ClientTickPreEvent) {
        if self.close.swap(false, Ordering::Relaxed) {
            host.mod_screen_close(ID);
        }
    }

    fn screen_render_post(&self, host: &dyn ClientHost, event: &mut ScreenRenderPostEvent<'_>) {
        if host.mod_screen() != Some(ID) {
            return;
        }
        let [width, _] = event.draw.size();
        let mut screen = self.screen.lock().unwrap();
        let screen = screen.get_or_insert_with(|| Screen::new(width));
        event.draw.fill([0.0, 30.0, width, 100.0], 0x8000_0000);
        // A refused label is not drawn; screen input remains usable.
        let _ = screen
            .widgets
            .draw(&mut *event.draw, host, event.screen.mouse);
    }

    fn screen_mouse_button_pressed_pre(
        &self,
        host: &dyn ClientHost,
        event: &mut ScreenMouseButtonPressedPreEvent<'_>,
    ) -> Flow {
        let (at, button, modifiers) = (event.screen.mouse, event.button, event.modifiers);
        self.input(host, |w| w.mouse_pressed(host, at, button, modifiers))
    }

    fn screen_mouse_button_released_pre(
        &self,
        host: &dyn ClientHost,
        event: &mut ScreenMouseButtonReleasedPreEvent<'_>,
    ) -> Flow {
        let button = event.button;
        self.input(host, |w| w.mouse_released(button))
    }

    fn screen_mouse_dragged_pre(
        &self,
        host: &dyn ClientHost,
        event: &mut ScreenMouseDraggedPreEvent<'_>,
    ) -> Flow {
        let (at, button) = (event.screen.mouse, event.button);
        self.input(host, |w| w.mouse_dragged(host, at, button))
    }

    fn screen_key_pressed_pre(
        &self,
        host: &dyn ClientHost,
        event: &mut ScreenKeyPressedPreEvent<'_>,
    ) -> Flow {
        let (key, modifiers) = (event.key, event.modifiers);
        self.input(host, |w| w.key_pressed(host, key, modifiers))
    }

    fn screen_char_typed_pre(
        &self,
        host: &dyn ClientHost,
        event: &mut ScreenCharTypedPreEvent<'_>,
    ) -> Flow {
        let c = event.c;
        self.input(host, |w| w.char_typed(host, c))
    }
}
// ANCHOR_END: widget_screen
