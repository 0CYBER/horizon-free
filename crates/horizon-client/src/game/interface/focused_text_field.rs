//! The focused text box as `native_screen_char_typed_pre` hands it to a mod: a copy of its
//! `TextEdit`, whose edits go back through the box's own filter, length limit and responders.
//! The boxes with validators of their own (Options, World Options, the game rules, the block
//! editor) are not offered.
use super::{Game, Screen, control};
use horizon_client::{
    chat::Commands,
    screen_draw::{Measure, TextField},
    ui::EditBox,
};
use mod_api::client::widgets::TextEdit;

/// `edited`'s text and caret into `field`: the text as if typed over all of it, so the box's
/// filter and limit apply, and the positions kept within what it took.
fn edit_box(field: &mut EditBox, edited: &TextEdit) {
    if field.value != edited.value {
        field.select_all();
        field.insert_text(&edited.value);
    }
    field.set_cursor(edited.highlight, false);
    field.set_cursor(edited.cursor, true);
}

/// As `edit_box`, for the boxes that measure their text.
fn edit_text_field(field: &mut TextField, measure: &Measure<'_>, edited: &TextEdit) {
    if field.value != edited.value {
        field.move_cursor_to(measure, usize::MAX, false);
        field.move_cursor_to(measure, 0, true);
        field.insert_text(measure, &edited.value);
    }
    field.move_cursor_to(measure, edited.highlight, false);
    field.move_cursor_to(measure, edited.cursor, true);
}

/// The box's state, after `edited` if there is one.
fn visit_box(field: &mut EditBox, edited: Option<&TextEdit>) -> TextEdit {
    if let Some(edited) = edited {
        edit_box(field, edited);
    }
    field.edit.clone()
}

impl Game<'_> {
    /// The box a typed character goes to (`type_chars_into_screen`'s choice), as it stands
    /// after `edited` when there is one, whose responders then run.
    fn focused_text_edit(&mut self, edited: Option<&TextEdit>) -> Option<TextEdit> {
        if control::title_owns_text(&self.screen, self.title.is_some()) {
            let mut state = None;
            self.title
                .as_mut()?
                .edit_focused_text_field(|field| state = Some(visit_box(field, edited)));
            return state;
        }
        match self.screen {
            Screen::Chat | Screen::CommandBlock => {
                let measure = Measure(&self.gui.atlas.font);
                let commands = Commands {
                    dispatcher: super::chat::chat_tree(&self.remote),
                    registries: &self.ui.registries,
                };
                let screen = if self.screen == Screen::Chat {
                    self.ui.chat_screen.as_mut()?
                } else {
                    &mut self.command_block_screen.as_mut()?.command
                };
                if let Some(edited) = edited {
                    screen.edit_input(&measure, &commands, |field| {
                        edit_text_field(field, &measure, edited)
                    });
                }
                let field = &screen.input;
                let mut state = TextEdit::new(field.max_length);
                state.value.clone_from(&field.value);
                state.cursor = field.cursor;
                state.highlight = field.highlight;
                Some(state)
            }
            Screen::Social => {
                if edited.is_some() {
                    self.social.scroll = 0.0;
                }
                Some(visit_box(&mut self.social.search, edited))
            }
            Screen::DebugOptions => self
                .debug_options
                .as_mut()
                .filter(|screen| screen.search.focused)
                .map(|screen| visit_box(&mut screen.search, edited)),
            Screen::Inventory | Screen::Creative if self.mod_browser_typing() => {
                let state = visit_box(self.panel.search.as_mut()?, edited);
                if edited.is_some() {
                    self.mod_browser_text(None, false);
                }
                Some(state)
            }
            Screen::Inventory if self.recipe_search_focused() => {
                Some(visit_box(self.recipe_ui.search.as_mut()?, edited))
            }
            Screen::Inventory if self.station_typing() => {
                let state = visit_box(&mut self.station.as_mut()?.rename, edited);
                if edited.is_some() {
                    self.station_text(None, false);
                }
                Some(state)
            }
            Screen::Creative if self.creative_screen.tab().search && !self.creative_ignore_text => {
                if edited.is_some() {
                    self.creative_screen.scroll = 0.0;
                }
                Some(visit_box(&mut self.creative_screen.search, edited))
            }
            _ => None,
        }
    }
    /// `hear` with a copy of the focused box, `None` with no box to offer; what it leaves of
    /// the copy goes back to the box. `hear`'s answer is handed back.
    pub(super) fn with_focused_text_edit(
        &mut self,
        hear: impl FnOnce(&Self, &mut Option<TextEdit>) -> bool,
    ) -> bool {
        let before = self.focused_text_edit(None);
        let mut field = before.clone();
        let answer = hear(self, &mut field);
        if before.is_some()
            && field != before
            && let Some(edited) = &field
        {
            self.focused_text_edit(Some(edited));
        }
        answer
    }
}

#[cfg(test)]
mod tests {
    use super::super::native_entity_refs::tests::with_client_game;
    use super::super::native_hooks::particle_count_tests::frame_scene;
    use super::*;
    use horizon_client::chat::{ChatAbilities, ChatScreen, ChatVisibility};
    use horizon_client::{multiplayer, title};
    use mod_api::{ClientHost, Flow, Hooks, ModEntry, ScreenCharTypedPreEvent};
    use std::sync::{Arc, Mutex};
    use winit::keyboard::KeyCode;

    /// Writes down each character and the box it saw, and edits the box on a few of them.
    struct Editor(Arc<Mutex<Vec<String>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Editor {
        fn screen_char_typed_pre(
            &self,
            _: &dyn ClientHost,
            event: &mut ScreenCharTypedPreEvent<'_>,
        ) -> Flow {
            let c = event.c;
            let Some(field) = event.field.as_mut() else {
                self.0.lock().unwrap().push(format!("{c}"));
                *event.field = Some(TextEdit::new(5));
                return Flow::Continue;
            };
            self.0.lock().unwrap().push(format!(
                "{c}:{}:{}:{}",
                field.value, field.cursor, field.highlight
            ));
            match c {
                '?' => Flow::Cancel,
                '!' => {
                    field.value = "a\u{1f600}b".into();
                    field.cursor = 2;
                    field.highlight = 2;
                    field.max_length = 99;
                    Flow::Cancel
                }
                '#' => {
                    field.value = "§a\nbcdefghijklmnopqrstuvwxyz".into();
                    Flow::Cancel
                }
                '@' => {
                    field.value = "\u{1f600} hi st".into();
                    field.cursor = 7;
                    field.highlight = 7;
                    Flow::Cancel
                }
                '^' => {
                    field.value = "z".into();
                    Flow::Continue
                }
                '~' => {
                    *event.field = None;
                    Flow::Cancel
                }
                _ => Flow::Continue,
            }
        }
    }

    /// The hook and the box through the game's own text paths, on a real window's screens:
    /// each character sees what the one before it left, a cancelled one is not typed and
    /// its edit stays, and the box's filter, limit and responders hold.
    #[test]
    #[ignore = "daily"]
    fn a_mod_edits_the_focused_box_before_each_character() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let taken = || std::mem::take(&mut *seen.lock().unwrap());
        let mods: mod_api::Mods =
            vec![ModEntry::new("editor", Box::new(Editor(seen.clone())))].into();
        mod_platform::testing::with_mods(mods, || {
            with_client_game(|game| {
                let _event_loop = frame_scene(game);
                let social = |game: &Game<'_>| {
                    let search = &game.social.search;
                    (search.value.clone(), search.cursor, search.highlight)
                };

                // An input method's commit, over a selected emoji.
                game.screen = Screen::Social;
                game.social.search.set_value("a\u{1f600}b");
                game.social.search.set_cursor(2, false);
                game.social.search.set_cursor(1, true);
                game.ime_commit("x?!");
                assert_eq!(taken(), ["x:a\u{1f600}b:1:2", "?:axb:2:2", "!:axb:2:2",]);
                assert_eq!(social(game), ("a\u{1f600}b".to_owned(), 2, 2));
                assert_eq!(game.social.search.max_length, 16);

                // A key's characters: the box's own filter and limit cut a replacement.
                game.screen_chars_typed(Some("#"));
                assert_eq!(game.social.search.value, "abcdefghijklmnop");
                taken();

                // An edit that lets the character through: it lands at the edited caret.
                game.social.search.set_value("");
                game.screen_chars_typed(Some("^"));
                assert_eq!(taken(), ["^::0:0"]);
                assert_eq!(game.social.search.value, "^z");

                // Clearing `field` is no edit of the box.
                game.screen_chars_typed(Some("~"));
                assert_eq!(game.social.search.value, "^z");
                taken();

                // The chat box: its completions follow the replaced text.
                game.screen = Screen::Chat;
                game.ui.registries.players = vec!["Steve".into(), "Steven".into()];
                game.ui.chat_screen = Some(ChatScreen::new(
                    &Measure(&game.gui.atlas.font),
                    &Commands {
                        dispatcher: super::super::chat::chat_tree(&game.remote),
                        registries: &game.ui.registries,
                    },
                    "",
                    false,
                    0,
                    640,
                    360,
                    ChatAbilities::new(ChatVisibility::Full),
                ));
                game.screen_chars_typed(Some("@"));
                assert_eq!(taken(), ["@::0:0"]);
                game.chat_key_code(KeyCode::Tab, None);
                game.chat_key_code(KeyCode::Tab, None);
                assert_eq!(
                    game.ui.chat_screen.as_ref().unwrap().value(),
                    "\u{1f600} hi Steve"
                );

                // No box to offer: the mod's `Some` goes nowhere.
                game.screen = Screen::Inventory;
                game.ui.chat_screen = None;
                game.screen_chars_typed(Some("q"));
                assert_eq!(taken(), ["q"]);

                // A prompt over the screen keeps the box and the hook away.
                game.screen = Screen::Social;
                game.link_prompt = Some("https://example.com".into());
                game.ime_commit("w");
                assert_eq!(taken(), [""; 0]);
                game.link_prompt = None;

                // The title's own field.
                game.screen = Screen::Playing;
                let mut menu = title::Title::new(
                    std::path::PathBuf::from("/nonexistent-field-fixture/saves"),
                    std::path::Path::new("/nonexistent-field-fixture"),
                    "",
                );
                menu.mode = title::Mode::Multiplayer(multiplayer::Screen::Direct {
                    address: "\u{1f600}".into(),
                });
                game.title = Some(menu);
                game.screen_chars_typed(Some("y?"));
                assert_eq!(taken(), ["y:\u{1f600}:1:1", "?:\u{1f600}y:2:2"]);
                let (_, value, _) = game.title.as_mut().unwrap().focused_field().unwrap();
                assert_eq!(value, "\u{1f600}y");
            });
        });
    }

    /// A mod's replacement is typed, not assigned: the box's filter and limit hold, its
    /// positions count scalars, and its limit is not the mod's to raise.
    #[test]
    fn a_replacement_keeps_the_box_filter_limit_and_scalar_positions() {
        let mut field = EditBox::new(0.0, 0.0, 100.0, 20.0);
        field.max_length = 4;
        field.set_value("a\u{1f600}b");
        let mut edited = field.edit.clone();
        edited.value = "a\n§\u{1f600}bcdef".into();
        edited.cursor = usize::MAX;
        edited.highlight = 1;
        edited.max_length = 100;
        let state = visit_box(&mut field, Some(&edited));
        // The limit counts UTF-16 units, while selection positions count scalars.
        assert_eq!(state.value, "a\u{1f600}b");
        assert_eq!((state.cursor, state.highlight), (3, 1));
        assert_eq!(state.max_length, 4);
    }

    /// Only the caret moved: the text is left alone, even one the filter would refuse.
    #[test]
    fn a_caret_move_leaves_the_text() {
        let mut field = EditBox::new(0.0, 0.0, 100.0, 20.0);
        field.value = "a§b".into();
        let mut edited = field.edit.clone();
        edited.cursor = 1;
        edited.highlight = 3;
        let state = visit_box(&mut field, Some(&edited));
        assert_eq!(state.value, "a§b");
        assert_eq!((state.cursor, state.highlight), (1, 3));
    }
}
