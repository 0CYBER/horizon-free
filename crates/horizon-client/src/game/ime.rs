//! Input method (IME) text: committed composition goes to the focused text
//! field through the same sinks the keyboard and `horizon-ctl type` use.
//! The window only asks the OS for an input method while such a field has
//! focus, so movement keys keep arriving as plain key events.
use super::{Game, Screen, books::BookScreen};

impl Game<'_> {
    /// Whether a text box has focus right now (the fields `ime_commit` can fill).
    pub(super) fn text_field_focused(&mut self) -> bool {
        if self.dialog_open() {
            return self.dialog_has_text_field();
        }
        if let Some(title) = self.title.as_mut() {
            return title.focused_field().is_some();
        }
        if self.sign_edit.is_some() || matches!(self.book, Some(BookScreen::Edit { .. })) {
            return true;
        }
        match self.screen {
            Screen::Chat | Screen::CommandBlock => true,
            Screen::WorldOptions => self.world_options_typing(),
            Screen::Creative => self.creative_screen.tab().search,
            Screen::Inventory => {
                self.mod_browser_typing() || self.recipe_search_focused() || self.station_typing()
            }
            _ => false,
        }
    }

    /// Allow the input method only while a text box has focus.
    pub(super) fn sync_ime(&mut self) {
        let want = self.text_field_focused();
        if want != self.ime_allowed
            && let Some(window) = &self.window
        {
            window.set_ime_allowed(want);
            self.ime_allowed = want;
        }
    }

    /// Text the input method committed (`WindowEvent::Ime(Commit)`).
    pub(super) fn ime_commit(&mut self, text: &str) {
        let text: String = text.chars().filter(|c| !c.is_control()).collect();
        if text.is_empty() || !self.text_field_focused() {
            return;
        }
        if self.dialog_type(&text) {
            return;
        }
        if self.title.is_some() {
            self.title_edit(Some(&text), false);
        } else if self.book.is_some() {
            self.book_input(None, Some(&text));
        } else if self.sign_edit.is_some() {
            self.sign_input(None, Some(&text));
        } else {
            match self.screen {
                Screen::Chat => self.chat_type(&text),
                Screen::CommandBlock => self.command_block_type(&text),
                Screen::WorldOptions => self.world_options_text(Some(&text)),
                Screen::Creative => self.creative_search_edit(Some(&text)),
                Screen::Inventory if self.mod_browser_typing() => {
                    self.mod_browser_text(Some(&text), false)
                }
                Screen::Inventory if self.recipe_search_focused() => {
                    self.recipe_text(Some(&text), false)
                }
                Screen::Inventory => self.station_text(Some(&text), false),
                _ => {}
            }
        }
    }
}
