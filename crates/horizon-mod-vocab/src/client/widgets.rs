//! Buttons and text fields for a mod's own screens (`ClientHost::mod_screen_open`), kept by the
//! mod and driven by the screen hooks it already listens to: draw them in
//! `screen_render_post`, and hand them the presses, releases, drags, keys and typed characters of
//! `screen_mouse_button_pressed_pre`, `screen_mouse_button_released_pre`,
//! `screen_mouse_dragged_pre`, `screen_key_pressed_pre` and `screen_char_typed_pre`. Each input
//! call answers an [`Outcome`](crate::client::widgets::Outcome): set the hook's `canceled` when it
//! [`Outcome::handled`](crate::client::widgets::Outcome::handled) the input.
//!
//! They behave as vanilla's `Button` and `EditBox` do: the game draws them with its own sprites,
//! label and caret (`DrawContext::widget_button`, `DrawContext::widget_text_field`), so they get
//! its narration and controller support, and a text field edits with the same core as the
//! game's own ([`TextEdit`](crate::TextEdit)). Everything else is plain Rust in the mod,
//! the same in a native and a wasm build.
//!
//! ```
//! use mod_api::client::widgets::{Button, Outcome, TextField, Widgets};
//! use mod_api::Text;
//! let mut widgets = Widgets::new();
//! let name = widgets.add_text_field(TextField::new([20.0, 20.0, 150.0, 20.0], 32));
//! let done = widgets.add_button(Button::new([20.0, 50.0, 150.0, 20.0], Text::literal("Done")));
//! widgets.text_field_mut(name).unwrap().set_value("Steve");
//! assert_eq!(widgets.text_field(name).unwrap().value(), "Steve");
//! assert!(widgets.button(done).unwrap().active);
//! ```
use crate::{ClientHost, DrawContext, SoundPlay, Text};

#[path = "widgets_text_edit.rs"]
mod text_edit;
pub use text_edit::{
    EditKey, TextEdit, byte_index, char_len, filter_text, is_allowed_chat_character,
    java_whitespace, multiline_word_position, utf16_len, utf16_prefix, word_position,
};

/// GLFW's Shift bit in the screen hooks' `modifiers`.
const SHIFT: u32 = 1;
/// GLFW's Control bit.
const CONTROL: u32 = 2;
/// GLFW's Super bit: the edit modifier on macOS (`Minecraft.ON_OSX`).
const SUPER: u32 = 8;
/// `MouseHandler`'s double click: a second press of the same button within 250 ms.
const DOUBLE_CLICK_MS: u64 = 250;
/// `EditBox`'s caret blinks on and off every 300 ms from when it last moved.
const BLINK_MS: u64 = 300;
/// `EditBox.textColor` and `textColorUneditable`.
const TEXT_COLOR: u32 = 0xFFE0_E0E0;
const TEXT_COLOR_INACTIVE: u32 = 0xFF70_7070;

/// A widget of a [`Widgets`] group, as it answered when added.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
// exhaustive: an index into its group; mods only compare and pass it back
#[derive(horizon_mod_api_macros::Wire)]
pub struct WidgetId(usize);

/// What a widget made of an input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum Outcome {
    /// No widget took it: leave the hook's `canceled` alone.
    Ignored,
    /// A widget took it (a focus change, a caret move, a drag) with nothing else to report.
    Handled,
    /// The button was pressed: a left click, or Enter or Space while it has the focus.
    Pressed(WidgetId),
    /// The text field's text changed.
    Changed(WidgetId),
    /// Enter was pressed in the text field.
    Submitted(WidgetId),
}

impl Outcome {
    /// Whether a widget took the input, so the hook should set `canceled`.
    pub fn handled(self) -> bool {
        self != Self::Ignored
    }
}

/// A push button (`Button`): a label on the game's `widget/button` sprite.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct Button {
    /// Where it is, `[x, y, width, height]` in GUI pixels (vanilla's are 20 high).
    pub rect: [f32; 4],
    /// Its label, centred and scrolled when it doesn't fit.
    pub label: Text,
    /// An inactive button is greyed out, takes no press and is skipped by Tab.
    pub active: bool,
}

impl Button {
    /// An active button.
    pub fn new(rect: [f32; 4], label: Text) -> Self {
        Self {
            rect,
            label,
            active: true,
        }
    }
}

/// A single-line text field (`EditBox`).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct TextField {
    /// Where it is, `[x, y, width, height]` in GUI pixels (vanilla's are 20 high).
    pub rect: [f32; 4],
    /// The text, cursor and selection.
    pub edit: TextEdit,
    /// An inactive field shows its text greyed out, takes no input and is skipped by Tab.
    pub active: bool,
    /// Drawn on the `widget/text_field` sprite with a 4-pixel margin; without, the bare text.
    pub bordered: bool,
    /// The text's colour, ARGB (`EditBox.setTextColor`).
    pub text_color: u32,
    /// Grey text shown after the end of the text (`EditBox.setSuggestion`).
    pub suggestion: Option<String>,
    /// What the narrator calls it (`EditBox`'s message: `<message> edit box: <text>`).
    pub message: String,
    /// The first character shown, as the game left it after the last draw.
    scroll: usize,
    /// The x of each shown character boundary after the last draw.
    edges: Vec<f32>,
    /// `host.millis()` when the caret last moved: it blinks from there.
    moved_at: u64,
}

impl TextField {
    /// An empty, active, bordered field that takes up to `max_length` characters.
    pub fn new(rect: [f32; 4], max_length: usize) -> Self {
        Self {
            rect,
            edit: TextEdit::new(max_length),
            active: true,
            bordered: true,
            text_color: TEXT_COLOR,
            suggestion: None,
            message: String::new(),
            scroll: 0,
            edges: Vec::new(),
            moved_at: 0,
        }
    }

    /// The text.
    pub fn value(&self) -> &str {
        &self.edit.value
    }

    /// `EditBox.setValue`: the text, cut to the maximum length, with the cursor at its end.
    pub fn set_value(&mut self, value: &str) {
        self.edit.set_value(value);
        self.scroll_to_cursor();
    }

    /// `EditBox.scrollTo` without the font: back to the cursor when it is left of what shows,
    /// and about a field's width back when it is on the first shown character (all the way, and
    /// the game scrolls forward until the cursor is at the right edge when it draws).
    fn scroll_to_cursor(&mut self) {
        self.scroll = self.scroll.min(char_len(&self.edit.value));
        if self.edit.cursor == self.scroll && self.scroll > 0 {
            self.scroll = 0;
        } else if self.edit.cursor < self.scroll {
            self.scroll = self.edit.cursor;
        }
    }

    /// `EditBox.findClickedPositionInText`: the character boundary under GUI `x`, from the
    /// boundaries the last draw left.
    fn position_at(&self, x: f32) -> usize {
        let x = x.floor();
        let shown = self
            .edges
            .iter()
            .skip(1)
            .take_while(|&&edge| edge <= x)
            .count();
        self.scroll + shown
    }

    /// The caret moved: scroll back to it and restart its blink.
    fn moved(&mut self, now: u64) {
        self.scroll_to_cursor();
        self.moved_at = now;
    }

    /// What the game draws.
    fn view(&self, focused: bool, now: u64) -> TextFieldView {
        TextFieldView {
            rect: self.rect,
            value: self.edit.value.clone(),
            cursor: self.edit.cursor,
            highlight: self.edit.highlight,
            max_length: self.edit.max_length,
            scroll: self.scroll,
            focused,
            caret_shown: focused
                && (now.saturating_sub(self.moved_at) / BLINK_MS).is_multiple_of(2),
            bordered: self.bordered,
            text_color: if self.active {
                self.text_color
            } else {
                TEXT_COLOR_INACTIVE
            },
            suggestion: self.suggestion.clone(),
            message: self.message.clone(),
        }
    }
}

/// A text field as [`DrawContext::widget_text_field`] draws it: the game's `EditBox` look,
/// filled in by [`Widgets`].
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct TextFieldView {
    /// Where it is, `[x, y, width, height]` in GUI pixels.
    pub rect: [f32; 4],
    /// The text.
    pub value: String,
    /// The cursor and the other end of the selection, in characters.
    pub cursor: usize,
    /// The selection's anchor.
    pub highlight: usize,
    /// The field's maximum length: a full field shows a bar for its caret.
    pub max_length: usize,
    /// The first character shown, from the last draw.
    pub scroll: usize,
    /// It has the keyboard focus: highlighted sprite, caret, narration.
    pub focused: bool,
    /// The caret is in the visible half of its blink.
    pub caret_shown: bool,
    /// On the `widget/text_field` sprite with a 4-pixel margin.
    pub bordered: bool,
    /// The text's colour, ARGB.
    pub text_color: u32,
    /// Grey text after the end of the text.
    pub suggestion: Option<String>,
    /// The narrator's name for it.
    pub message: String,
}

/// What [`DrawContext::widget_text_field`] drew: the first character shown (after scrolling to
/// the cursor) and the GUI x of each shown character boundary, from the start of the shown
/// text to its end, so a press maps to the character it is on.
#[derive(Clone, Debug, PartialEq, Default)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct TextFieldLayout {
    /// The first character shown.
    pub scroll: usize,
    /// The x of each shown character boundary, left to right.
    pub edges: Vec<f32>,
}

impl TextFieldLayout {
    /// The layout a draw answers.
    pub fn new(scroll: usize, edges: Vec<f32>) -> Self {
        Self { scroll, edges }
    }
}

/// A widget of a group.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum Widget {
    /// A button.
    Button(Button),
    /// A text field.
    TextField(TextField),
}

impl Widget {
    fn rect(&self) -> [f32; 4] {
        match self {
            Self::Button(button) => button.rect,
            Self::TextField(field) => field.rect,
        }
    }

    fn active(&self) -> bool {
        match self {
            Self::Button(button) => button.active,
            Self::TextField(field) => field.active,
        }
    }
}

/// `AbstractWidget.isMouseOver`.
fn over([x, y, w, h]: [f32; 4], [px, py]: [f32; 2]) -> bool {
    px >= x && py >= y && px < x + w && py < y + h
}

/// The widgets of one screen in focus order, and which one has the keyboard focus.
#[derive(Clone, Debug, Default, PartialEq)]
// exhaustive: every field is private; mods build it with `Widgets::new`
#[derive(horizon_mod_api_macros::Wire)]
pub struct Widgets {
    widgets: Vec<Widget>,
    focus: Option<usize>,
    /// The left button went down on the focused widget and is still held (`Screen.isDragging`).
    dragging: bool,
    /// The last left press's time, for double clicks.
    last_press: Option<u64>,
}

impl Widgets {
    /// An empty group.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds `button` after the others; Tab reaches it in that order.
    pub fn add_button(&mut self, button: Button) -> WidgetId {
        self.widgets.push(Widget::Button(button));
        WidgetId(self.widgets.len() - 1)
    }

    /// Adds `field` after the others; Tab reaches it in that order.
    pub fn add_text_field(&mut self, field: TextField) -> WidgetId {
        self.widgets.push(Widget::TextField(field));
        WidgetId(self.widgets.len() - 1)
    }

    /// The widget `id`.
    pub fn widget(&self, id: WidgetId) -> Option<&Widget> {
        self.widgets.get(id.0)
    }

    /// Button `id`, if it is one.
    pub fn button(&self, id: WidgetId) -> Option<&Button> {
        match self.widgets.get(id.0) {
            Some(Widget::Button(button)) => Some(button),
            _ => None,
        }
    }

    /// Button `id`, to change its label or make it inactive.
    pub fn button_mut(&mut self, id: WidgetId) -> Option<&mut Button> {
        match self.widgets.get_mut(id.0) {
            Some(Widget::Button(button)) => Some(button),
            _ => None,
        }
    }

    /// Text field `id`, if it is one.
    pub fn text_field(&self, id: WidgetId) -> Option<&TextField> {
        match self.widgets.get(id.0) {
            Some(Widget::TextField(field)) => Some(field),
            _ => None,
        }
    }

    /// Text field `id`, to read or set its text.
    pub fn text_field_mut(&mut self, id: WidgetId) -> Option<&mut TextField> {
        match self.widgets.get_mut(id.0) {
            Some(Widget::TextField(field)) => Some(field),
            _ => None,
        }
    }

    /// The widget with the keyboard focus.
    pub fn focused(&self) -> Option<WidgetId> {
        self.focus.map(WidgetId)
    }

    /// Gives `id` the keyboard focus (an inactive widget can't take it), or clears it.
    pub fn set_focus(&mut self, host: &dyn ClientHost, id: Option<WidgetId>) {
        self.focus_index(id.map(|id| id.0), host.millis());
    }

    /// The focused widget while it is active: a widget made inactive after taking the focus
    /// keeps it (Tab moves on from there) but takes no input (`AbstractButton.keyPressed`,
    /// `EditBox.canConsumeInput`).
    fn active_focus(&self) -> Option<usize> {
        self.focus
            .filter(|&i| self.widgets.get(i).is_some_and(Widget::active))
    }

    fn focus_index(&mut self, index: Option<usize>, now: u64) {
        let index = index.filter(|&i| self.widgets.get(i).is_some_and(Widget::active));
        if index != self.focus
            && let Some(Widget::TextField(field)) = index.and_then(|i| self.widgets.get_mut(i))
        {
            field.moved_at = now;
        }
        self.focus = index;
    }

    /// Draws every widget, the one under `mouse` (the screen's `mouse`) or with the focus
    /// highlighted, in the order added. Returns a refusal if a label cannot cross the text boundary.
    pub fn draw(
        &mut self,
        draw: &mut dyn DrawContext,
        host: &dyn ClientHost,
        mouse: [f32; 2],
    ) -> Result<(), crate::text::TextFormError> {
        let now = host.millis();
        for (index, widget) in self.widgets.iter_mut().enumerate() {
            let focused = self.focus == Some(index);
            match widget {
                Widget::Button(button) => {
                    let highlighted = button.active && (focused || over(button.rect, mouse));
                    draw.widget_button(button.rect, &button.label, button.active, highlighted)?;
                }
                Widget::TextField(field) => {
                    let layout = draw.widget_text_field(&field.view(focused, now));
                    field.scroll = layout.scroll;
                    field.edges = layout.edges;
                }
            }
        }
        Ok(())
    }

    /// A mouse button went down at `at` (the screen's `mouse`): `Screen.mouseClicked`. Only the
    /// left button clicks widgets (`isValidClickButton`). The widget under it takes the focus
    /// and a button is pressed (with its click sound), or a field's cursor goes there, Shift
    /// extending the selection and a double click selecting the word. A left press over no
    /// widget clears the focus and is `Ignored`.
    pub fn mouse_pressed(
        &mut self,
        host: &dyn ClientHost,
        at: [f32; 2],
        button: i32,
        modifiers: u32,
    ) -> Outcome {
        if button != 0 {
            return Outcome::Ignored;
        }
        let now = host.millis();
        let double = self
            .last_press
            .is_some_and(|then| now.saturating_sub(then) < DOUBLE_CLICK_MS);
        let Some(index) = self
            .widgets
            .iter()
            .position(|widget| widget.active() && over(widget.rect(), at))
        else {
            self.focus_index(None, now);
            return Outcome::Ignored;
        };
        // `MouseHandler.onButton` keeps the time only of a click the screen took.
        self.last_press = Some(now);
        self.focus_index(Some(index), now);
        self.dragging = true;
        match &mut self.widgets[index] {
            Widget::Button(_) => {
                click_sound(host);
                Outcome::Pressed(WidgetId(index))
            }
            Widget::TextField(field) => {
                let pos = field.position_at(at[0]);
                if double {
                    let (start, end) = field.edit.word_at(pos);
                    field.edit.set_cursor(start, false);
                    field.edit.set_cursor(end, true);
                } else {
                    field.edit.set_cursor(pos, modifiers & SHIFT != 0);
                }
                field.moved(now);
                Outcome::Handled
            }
        }
    }

    /// A mouse button went up: a drag that began on a widget ends.
    pub fn mouse_released(&mut self, button: i32) -> Outcome {
        if button == 0 && std::mem::take(&mut self.dragging) {
            Outcome::Handled
        } else {
            Outcome::Ignored
        }
    }

    /// The pointer moved to `at` with `button` held (`EditBox.onDrag`): the selection of the
    /// focused field the press began on follows it.
    pub fn mouse_dragged(&mut self, host: &dyn ClientHost, at: [f32; 2], button: i32) -> Outcome {
        if button != 0 || !self.dragging {
            return Outcome::Ignored;
        }
        let now = host.millis();
        match self.active_focus().and_then(|i| self.widgets.get_mut(i)) {
            Some(Widget::TextField(field)) => {
                let pos = field.position_at(at[0]);
                field.edit.set_cursor(pos, true);
                field.moved(now);
                Outcome::Handled
            }
            Some(_) => Outcome::Handled,
            None => Outcome::Ignored,
        }
    }

    /// A key went down (or repeated), named as `screen_key_pressed_pre` names it
    /// (`key.keyboard.tab`). Tab and Shift+Tab move the focus in the order added
    /// (`FocusNavigationEvent.TabNavigation`, wrapping round); Enter or Space presses the
    /// focused button; the focused field edits with Backspace, Delete, the arrows, Home and End
    /// (Ctrl for a word, Shift to select), Ctrl+A/C/V/X through the clipboard, and answers
    /// Enter with [`Outcome::Submitted`]. Escape and other keys are `Ignored`.
    pub fn key_pressed(&mut self, host: &dyn ClientHost, key: &str, modifiers: u32) -> Outcome {
        let now = host.millis();
        let shift = modifiers & SHIFT != 0;
        let ctrl =
            modifiers & CONTROL != 0 || (cfg!(target_os = "macos") && modifiers & SUPER != 0);
        if key == "key.keyboard.tab" {
            return self.tab(!shift, now);
        }
        let Some(index) = self.active_focus() else {
            return Outcome::Ignored;
        };
        let id = WidgetId(index);
        match &mut self.widgets[index] {
            Widget::Button(_) => match key {
                "key.keyboard.enter" | "key.keyboard.keypad.enter" | "key.keyboard.space" => {
                    click_sound(host);
                    Outcome::Pressed(id)
                }
                _ => Outcome::Ignored,
            },
            Widget::TextField(field) => {
                let before = field.edit.value.clone();
                let edit_key = match key {
                    "key.keyboard.enter" | "key.keyboard.keypad.enter" => {
                        return Outcome::Submitted(id);
                    }
                    "key.keyboard.backspace" => Some(EditKey::Backspace),
                    "key.keyboard.delete" => Some(EditKey::Delete),
                    "key.keyboard.left" => Some(EditKey::Left),
                    "key.keyboard.right" => Some(EditKey::Right),
                    "key.keyboard.home" => Some(EditKey::Home),
                    "key.keyboard.end" => Some(EditKey::End),
                    "key.keyboard.a" if ctrl => Some(EditKey::SelectAll),
                    _ => None,
                };
                let handled = match edit_key {
                    Some(edit_key) => field.edit.key(edit_key, ctrl, shift),
                    None if ctrl => match key {
                        "key.keyboard.c" => {
                            host.set_clipboard_text(&field.edit.selected_text());
                            true
                        }
                        "key.keyboard.x" => {
                            host.set_clipboard_text(&field.edit.selected_text());
                            field.edit.insert_text("");
                            true
                        }
                        "key.keyboard.v" => {
                            if let Some(text) = host.clipboard_text() {
                                field.edit.insert_text(&text);
                            }
                            true
                        }
                        _ => false,
                    },
                    None => false,
                };
                if !handled {
                    return Outcome::Ignored;
                }
                field.moved(now);
                if field.edit.value != before {
                    Outcome::Changed(id)
                } else {
                    Outcome::Handled
                }
            }
        }
    }

    /// A character was typed (`screen_char_typed_pre`): the focused field takes it
    /// (`EditBox.charTyped`) when it is one a chat line may hold.
    pub fn char_typed(&mut self, host: &dyn ClientHost, c: char) -> Outcome {
        let Some(index) = self.active_focus() else {
            return Outcome::Ignored;
        };
        let Some(Widget::TextField(field)) = self.widgets.get_mut(index) else {
            return Outcome::Ignored;
        };
        if !is_allowed_chat_character(c) {
            return Outcome::Ignored;
        }
        let before = field.edit.value.clone();
        field.edit.insert_text(c.encode_utf8(&mut [0; 4]));
        field.moved(host.millis());
        if field.edit.value != before {
            Outcome::Changed(WidgetId(index))
        } else {
            Outcome::Handled
        }
    }

    /// `FocusNavigationEvent.TabNavigation`: the next (or previous) active widget after the
    /// focused one, wrapping round; from no focus, the first (or the last).
    fn tab(&mut self, forward: bool, now: u64) -> Outcome {
        let count = self.widgets.len();
        if count == 0 {
            return Outcome::Ignored;
        }
        let start = match (self.focus, forward) {
            (Some(i), true) => i + 1,
            (Some(i), false) => i + count - 1,
            (None, true) => 0,
            (None, false) => count - 1,
        };
        let next = (0..count)
            .map(|step| {
                if forward {
                    (start + step) % count
                } else {
                    (start + count - step) % count
                }
            })
            .find(|&i| self.widgets[i].active());
        match next {
            Some(i) => {
                self.focus_index(Some(i), now);
                Outcome::Handled
            }
            None => Outcome::Ignored,
        }
    }
}

/// `AbstractButton.playDownSound`.
fn click_sound(host: &dyn ClientHost) {
    host.play_local_sound(&SoundPlay::ui("minecraft:ui.button.click"), 0);
}

#[cfg(test)]
#[path = "widgets_tests.rs"]
mod tests;
