use super::*;
use crate::prelude::*;
use crate::{Host, SoundRef};
use std::cell::{Cell, RefCell};

/// A client host with a clock the test moves, a clipboard and a record of the sounds played.
#[derive(Default)]
struct Fake {
    now: Cell<u64>,
    clipboard: RefCell<String>,
    sounds: RefCell<Vec<String>>,
}
impl Host for Fake {}
crate::impl_host_areas!(Fake);
impl ClientHost for Fake {}
crate::impl_client_host_areas!(Fake, except(client_input, effects));
impl ClientInputClientHost for Fake {
    fn millis(&self) -> u64 {
        self.now.get()
    }
    fn clipboard_text(&self) -> Option<String> {
        Some(self.clipboard.borrow().clone())
    }
    fn set_clipboard_text(&self, text: &str) -> bool {
        *self.clipboard.borrow_mut() = text.to_owned();
        true
    }
}
impl EffectsClientHost for Fake {
    fn play_local_sound(&self, sound: &SoundPlay, _: u32) -> Option<SoundRef> {
        self.sounds.borrow_mut().push(sound.identifier.clone());
        None
    }
}

/// What the widgets asked the game to draw: each button as (label, active, highlighted), each
/// field's view; every character 6 pixels wide, from 4 pixels in.
#[derive(Default)]
struct Recorder {
    buttons: Vec<(Text, bool, bool)>,
    fields: Vec<TextFieldView>,
}
impl DrawContext for Recorder {
    fn widget_button(
        &mut self,
        _: [f32; 4],
        label: &Text,
        active: bool,
        highlighted: bool,
    ) -> Result<(), crate::text::TextFormError> {
        label.validate_depth()?;
        self.buttons.push((label.clone(), active, highlighted));
        Ok(())
    }
    fn widget_text_field(&mut self, field: &TextFieldView) -> TextFieldLayout {
        self.fields.push(field.clone());
        let left = field.rect[0] + 4.0;
        let shown = char_len(&field.value) - field.scroll;
        TextFieldLayout::new(
            field.scroll,
            (0..=shown).map(|i| left + 6.0 * i as f32).collect(),
        )
    }
}

/// Two fields and two buttons, as the example screen has them.
fn screen() -> (Widgets, [WidgetId; 4]) {
    let mut widgets = Widgets::new();
    let a = widgets.add_text_field(TextField::new([10.0, 10.0, 100.0, 20.0], 32));
    let b = widgets.add_text_field(TextField::new([10.0, 40.0, 100.0, 20.0], 32));
    let copy = widgets.add_button(Button::new(
        [10.0, 70.0, 100.0, 20.0],
        Text::literal("Copy"),
    ));
    let done = widgets.add_button(Button::new(
        [10.0, 100.0, 100.0, 20.0],
        Text::literal("Done"),
    ));
    (widgets, [a, b, copy, done])
}

/// A press focuses the field under it and puts the cursor at the character it is on; typing,
/// Backspace and the clipboard edit it, each a `Changed`; a press over nothing clears the focus.
#[test]
fn a_field_takes_clicks_typing_and_the_clipboard() {
    let host = Fake::default();
    let (mut widgets, [a, ..]) = screen();
    let mut draw = Recorder::default();
    widgets.text_field_mut(a).unwrap().set_value("hello");
    widgets.draw(&mut draw, &host, [0.0, 0.0]).unwrap();
    // Character boundaries at 14, 20, 26...: x 27 is after the second character.
    assert_eq!(
        widgets.mouse_pressed(&host, [27.0, 15.0], 0, 0),
        Outcome::Handled
    );
    assert_eq!(widgets.focused(), Some(a));
    assert_eq!(widgets.text_field(a).unwrap().edit.cursor, 2);
    assert_eq!(widgets.char_typed(&host, 'X'), Outcome::Changed(a));
    assert_eq!(widgets.char_typed(&host, '\u{7}'), Outcome::Ignored);
    assert_eq!(widgets.text_field(a).unwrap().value(), "heXllo");
    assert_eq!(
        widgets.key_pressed(&host, "key.keyboard.backspace", 0),
        Outcome::Changed(a)
    );
    assert_eq!(
        widgets.key_pressed(&host, "key.keyboard.a", CONTROL),
        Outcome::Handled
    );
    assert_eq!(
        widgets.key_pressed(&host, "key.keyboard.c", CONTROL),
        Outcome::Handled
    );
    assert_eq!(*host.clipboard.borrow(), "hello");
    assert_eq!(
        widgets.key_pressed(&host, "key.keyboard.x", CONTROL),
        Outcome::Changed(a)
    );
    assert_eq!(widgets.text_field(a).unwrap().value(), "");
    assert_eq!(
        widgets.key_pressed(&host, "key.keyboard.v", CONTROL),
        Outcome::Changed(a)
    );
    assert_eq!(widgets.text_field(a).unwrap().value(), "hello");
    assert_eq!(
        widgets.key_pressed(&host, "key.keyboard.enter", 0),
        Outcome::Submitted(a)
    );
    assert_eq!(
        widgets.key_pressed(&host, "key.keyboard.escape", 0),
        Outcome::Ignored
    );
    assert_eq!(
        widgets.mouse_pressed(&host, [300.0, 300.0], 0, 0),
        Outcome::Ignored
    );
    assert_eq!(widgets.focused(), None);
    assert_eq!(widgets.char_typed(&host, 'y'), Outcome::Ignored);
}

/// A second press within 250 ms selects the word; Shift extends the selection; a drag from a
/// press on the field follows the pointer until the release.
#[test]
fn double_clicks_select_words_and_drags_select_text() {
    let host = Fake::default();
    let (mut widgets, [a, ..]) = screen();
    let mut draw = Recorder::default();
    widgets.text_field_mut(a).unwrap().set_value("ab cd");
    widgets.draw(&mut draw, &host, [0.0, 0.0]).unwrap();
    host.now.set(1000);
    widgets.mouse_pressed(&host, [39.0, 15.0], 0, 0);
    widgets.mouse_released(0);
    host.now.set(1100);
    widgets.mouse_pressed(&host, [39.0, 15.0], 0, 0);
    let edit = &widgets.text_field(a).unwrap().edit;
    assert_eq!((edit.highlight, edit.cursor), (3, 5));
    widgets.mouse_released(0);
    host.now.set(2000);
    widgets.mouse_pressed(&host, [14.0, 15.0], 0, 0);
    assert_eq!(
        widgets.mouse_dragged(&host, [27.0, 15.0], 0),
        Outcome::Handled
    );
    let edit = &widgets.text_field(a).unwrap().edit;
    assert_eq!((edit.highlight, edit.cursor), (0, 2));
    assert_eq!(widgets.mouse_released(0), Outcome::Handled);
    assert_eq!(
        widgets.mouse_dragged(&host, [33.0, 15.0], 0),
        Outcome::Ignored
    );
    host.now.set(3000);
    widgets.mouse_pressed(&host, [39.0, 15.0], 0, SHIFT);
    let edit = &widgets.text_field(a).unwrap().edit;
    assert_eq!((edit.highlight, edit.cursor), (0, 4));
    // The right button clicks no widget.
    assert_eq!(
        widgets.mouse_pressed(&host, [14.0, 15.0], 1, 0),
        Outcome::Ignored
    );
    assert_eq!(widgets.focused(), Some(a));
}

/// Tab walks the widgets in the order added and wraps round, Shift+Tab goes back, and an
/// inactive button is skipped; Enter and Space press the focused button with its click sound,
/// as a left click does.
#[test]
fn tab_moves_the_focus_and_keys_press_the_focused_button() {
    let host = Fake::default();
    let (mut widgets, [a, b, copy, done]) = screen();
    let tab = |widgets: &mut Widgets, modifiers| {
        widgets.key_pressed(&host, "key.keyboard.tab", modifiers);
        widgets.focused()
    };
    assert_eq!(tab(&mut widgets, 0), Some(a));
    assert_eq!(tab(&mut widgets, 0), Some(b));
    assert_eq!(tab(&mut widgets, 0), Some(copy));
    assert_eq!(tab(&mut widgets, 0), Some(done));
    assert_eq!(tab(&mut widgets, 0), Some(a));
    assert_eq!(tab(&mut widgets, SHIFT), Some(done));
    widgets.button_mut(copy).unwrap().active = false;
    assert_eq!(tab(&mut widgets, SHIFT), Some(b));
    assert_eq!(tab(&mut widgets, 0), Some(done));
    assert_eq!(
        widgets.key_pressed(&host, "key.keyboard.space", 0),
        Outcome::Pressed(done)
    );
    assert_eq!(
        widgets.key_pressed(&host, "key.keyboard.keypad.enter", 0),
        Outcome::Pressed(done)
    );
    assert_eq!(
        widgets.mouse_pressed(&host, [20.0, 75.0], 0, 0),
        Outcome::Ignored
    );
    assert_eq!(
        widgets.mouse_pressed(&host, [20.0, 105.0], 0, 0),
        Outcome::Pressed(done)
    );
    assert_eq!(
        *host.sounds.borrow(),
        ["minecraft:ui.button.click"; 3].map(String::from)
    );
    let mut empty = Widgets::new();
    assert_eq!(
        empty.key_pressed(&host, "key.keyboard.tab", 0),
        Outcome::Ignored
    );
}

/// The game draws each widget once a frame: the hovered or focused button highlighted, an
/// inactive one greyed, and the focused field with its caret blinking every 300 ms from its
/// last move; the scroll the game answers is kept for the next frame and the next press.
#[test]
fn drawing_asks_the_game_for_each_widget() {
    let host = Fake::default();
    let (mut widgets, [a, b, copy, done]) = screen();
    widgets.button_mut(done).unwrap().active = false;
    widgets.set_focus(&host, Some(b));
    let mut draw = Recorder::default();
    widgets.draw(&mut draw, &host, [20.0, 75.0]).unwrap();
    assert_eq!(
        draw.buttons,
        [
            (Text::literal("Copy"), true, true),
            (Text::literal("Done"), false, false)
        ]
    );
    assert_eq!(
        draw.fields
            .iter()
            .map(|f| (f.focused, f.caret_shown))
            .collect::<Vec<_>>(),
        [(false, false), (true, true)]
    );
    host.now.set(350);
    let mut draw = Recorder::default();
    widgets.draw(&mut draw, &host, [0.0, 0.0]).unwrap();
    assert!(!draw.fields[1].caret_shown);
    assert_eq!(widgets.char_typed(&host, 'z'), Outcome::Changed(b));
    let mut draw = Recorder::default();
    widgets.draw(&mut draw, &host, [0.0, 0.0]).unwrap();
    assert!(draw.fields[1].caret_shown, "typing restarts the blink");
    widgets.text_field_mut(a).unwrap().active = false;
    widgets.set_focus(&host, Some(a));
    assert_eq!(widgets.focused(), None, "an inactive field takes no focus");
    let mut draw = Recorder::default();
    widgets.draw(&mut draw, &host, [0.0, 0.0]).unwrap();
    assert_eq!(draw.fields[0].text_color, TEXT_COLOR_INACTIVE);
    assert_eq!(
        widgets.widget(copy).map(|w| w.rect()),
        Some([10.0, 70.0, 100.0, 20.0])
    );
}

/// A widget made inactive while it has the focus takes no more input: Enter on a button
/// presses nothing and plays nothing, typing and the edit keys leave a field as it is, and a
/// drag no longer selects (`AbstractButton.keyPressed`, `EditBox.canConsumeInput`). Tab still
/// moves on from it.
#[test]
fn a_focused_widget_made_inactive_takes_no_input() {
    let host = Fake::default();
    let (mut widgets, [a, b, copy, _]) = screen();
    let mut draw = Recorder::default();
    widgets.draw(&mut draw, &host, [0.0, 0.0]).unwrap();
    widgets.set_focus(&host, Some(copy));
    widgets.button_mut(copy).unwrap().active = false;
    assert_eq!(
        widgets.key_pressed(&host, "key.keyboard.enter", 0),
        Outcome::Ignored
    );
    assert!(host.sounds.borrow().is_empty());
    widgets.text_field_mut(a).unwrap().set_value("hello");
    widgets.mouse_pressed(&host, [14.0, 15.0], 0, 0);
    widgets.text_field_mut(a).unwrap().active = false;
    assert_eq!(widgets.char_typed(&host, 'x'), Outcome::Ignored);
    assert_eq!(
        widgets.key_pressed(&host, "key.keyboard.delete", 0),
        Outcome::Ignored
    );
    assert_eq!(
        widgets.key_pressed(&host, "key.keyboard.v", CONTROL),
        Outcome::Ignored
    );
    assert_eq!(
        widgets.mouse_dragged(&host, [33.0, 15.0], 0),
        Outcome::Ignored
    );
    let edit = &widgets.text_field(a).unwrap().edit;
    assert_eq!(
        (edit.value.as_str(), edit.highlight, edit.cursor),
        ("hello", 0, 0)
    );
    assert_eq!(
        widgets.key_pressed(&host, "key.keyboard.tab", 0),
        Outcome::Handled
    );
    assert_eq!(widgets.focused(), Some(b));
}

/// Replacing all of a scrolled field's text scrolls back to show it (`EditBox.scrollTo` steps
/// back when the cursor is on the first shown character), instead of leaving it out of view.
#[test]
fn replacing_a_scrolled_text_scrolls_back_to_it() {
    let host = Fake::default();
    let (mut widgets, [a, ..]) = screen();
    let mut draw = Recorder::default();
    widgets.set_focus(&host, Some(a));
    widgets.text_field_mut(a).unwrap().set_value("hello world");
    widgets.text_field_mut(a).unwrap().scroll = 6;
    widgets.key_pressed(&host, "key.keyboard.a", CONTROL);
    assert_eq!(widgets.char_typed(&host, 'x'), Outcome::Changed(a));
    widgets.draw(&mut draw, &host, [0.0, 0.0]).unwrap();
    assert_eq!(draw.fields[0].value, "x");
    assert_eq!(draw.fields[0].scroll, 0);
}

/// A press over no widget is not the first click of a double click (`MouseHandler.onButton`
/// keeps the time only of a click the screen took): a click on a field right after it only
/// places the cursor.
#[test]
fn an_ignored_press_starts_no_double_click() {
    let host = Fake::default();
    let (mut widgets, [a, ..]) = screen();
    let mut draw = Recorder::default();
    widgets.text_field_mut(a).unwrap().set_value("hello");
    widgets.draw(&mut draw, &host, [0.0, 0.0]).unwrap();
    host.now.set(1000);
    assert_eq!(
        widgets.mouse_pressed(&host, [300.0, 300.0], 0, 0),
        Outcome::Ignored
    );
    host.now.set(1100);
    widgets.mouse_pressed(&host, [27.0, 15.0], 0, 0);
    let edit = &widgets.text_field(a).unwrap().edit;
    assert_eq!((edit.highlight, edit.cursor), (2, 2));
}
