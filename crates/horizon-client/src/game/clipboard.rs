//! Copy and paste in the text fields: `KeyboardHandler`'s clipboard and the
//! shortcuts of `EditBox`, `MultiLineEditBox` and `TextFieldHelper`.
use super::*;
use horizon_client::clipboard::{self, EditShortcut};
use winit::event::KeyEvent;

impl Game<'_> {
    /// `InputWithModifiers.isSelectAll`/`isCopy`/`isPaste`/`isCut` of a key
    /// press (or repeat): the keyboard's modifier state, or a modifier key an
    /// agent holds.
    pub(super) fn edit_shortcut(&self, event: &KeyEvent) -> Option<EditShortcut> {
        if event.state != ElementState::Pressed {
            return None;
        }
        let m = self.modifiers;
        let held = |keys: [KeyCode; 2]| keys.iter().any(|k| self.keys.contains(k));
        // InputQuirks.EDIT_SHORTCUT_KEY_MODIFIER: Cmd on macOS, Ctrl elsewhere.
        let edit = if cfg!(target_os = "macos") {
            m.super_key() || held([KeyCode::SuperLeft, KeyCode::SuperRight])
        } else {
            m.control_key() || held([KeyCode::ControlLeft, KeyCode::ControlRight])
        };
        clipboard::edit_shortcut(
            clipboard::shortcut_letter(&event.logical_key, event.physical_key),
            edit,
            m.shift_key() || held([KeyCode::ShiftLeft, KeyCode::ShiftRight]),
            m.alt_key() || held([KeyCode::AltLeft, KeyCode::AltRight]),
        )
    }
    /// A shortcut in an append-only field limited to `max_chars` characters
    /// (an `EditBox` with `setMaxLength`). True when the value changed.
    pub(super) fn append_shortcut(
        selection: &mut clipboard::AppendSelection,
        board: &mut clipboard::SystemClipboard,
        id: clipboard::FieldId,
        value: &mut String,
        shortcut: EditShortcut,
        max_chars: usize,
    ) -> bool {
        selection.apply(id, value, shortcut, board, |v, t| {
            clipboard::append_limited(v, t, max_chars)
        })
    }
    /// Shortcuts on the title, world creation and server screens.
    pub(super) fn title_shortcut(&mut self, shortcut: EditShortcut) {
        let Some(title) = self.title.as_mut() else {
            return;
        };
        if let Some((id, value, max)) = title.focused_field() {
            Self::append_shortcut(
                &mut self.text_selection,
                &mut self.clipboard,
                id,
                value,
                shortcut,
                max,
            );
        }
    }
    /// Typing or a backspace in an append-only field of the title screens:
    /// Select All's selection is replaced first.
    pub(super) fn title_edit(&mut self, typed: Option<&str>, backspace: bool) {
        let Some(title) = self.title.as_mut() else {
            return;
        };
        let cleared = title
            .focused_field()
            .is_some_and(|(id, value, _)| self.text_selection.take(id, value));
        if backspace {
            if !cleared {
                title.backspace();
            }
        } else if let Some(text) = typed {
            title.type_text(text);
        }
    }
    /// Shortcuts in the creative inventory's search box (an `EditBox` of 50).
    pub(super) fn creative_search_shortcut(&mut self, shortcut: EditShortcut) {
        if Self::append_shortcut(
            &mut self.text_selection,
            &mut self.clipboard,
            ("creative.search", 0),
            &mut self.creative_screen.search,
            shortcut,
            CREATIVE_SEARCH_CHARS,
        ) {
            self.creative_screen.scroll = 0.0;
        }
    }
    /// Typing or a backspace in the creative search box.
    pub(super) fn creative_search_edit(&mut self, typed: Option<&str>, backspace: bool) {
        let search = &mut self.creative_screen.search;
        let cleared = self.text_selection.take(("creative.search", 0), search);
        if backspace {
            if !cleared {
                search.pop();
            }
        } else if let Some(text) = typed {
            clipboard::append_limited(search, text, CREATIVE_SEARCH_CHARS);
        }
        self.creative_screen.scroll = 0.0;
    }
}

/// `CreativeModeInventoryScreen`'s search box: `setMaxLength(50)`.
const CREATIVE_SEARCH_CHARS: usize = 50;
