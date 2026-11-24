use super::*;
use crate::game::{Audio, Options, Progress, Settings, load_resources};
use mod_api::{self, ChatSuggestionAppliedEvent, ChatSuggestionsEvent, ClientHost, Hooks};
use std::sync::{Arc, Mutex};

/// Exercise the actual Game keyboard/mouse handlers and generated mod dispatch.
#[test]
#[ignore = "daily"]
fn chat_completion_reaches_mods_from_keyboard_and_mouse() {
    crate::game::in_own_process(
        "game::chat::suggestion_completion_tests::chat_completion_reaches_mods_from_keyboard_and_mouse",
        chat_completion_reaches_mods_from_keyboard_and_mouse_isolated,
    );
}

fn chat_completion_reaches_mods_from_keyboard_and_mouse_isolated() {
    let root = PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    let scratch = PathBuf::from(std::env::var_os("HORIZON_HOME").unwrap()).join("suggestion-game");
    std::fs::create_dir_all(&scratch).unwrap();
    struct RestoreDir(PathBuf);
    impl Drop for RestoreDir {
        fn drop(&mut self) {
            std::env::set_current_dir(&self.0).unwrap();
        }
    }
    let _restore = RestoreDir(std::env::current_dir().unwrap());
    std::env::set_current_dir(&scratch).unwrap();
    let settings = Settings::default();
    let loaded = load_resources(
        horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        crate::game::execution_role::ExecutionRole::IntegratedServer,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root;
    let mut game = Game::new(
        &loaded.resources,
        loaded.pack,
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
            scratch.clone(),
        )),
        None,
    )
    .unwrap();

    struct Listener(Arc<Mutex<Vec<String>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Listener {
        fn chat_suggestion_applied(
            &self,
            _host: &dyn ClientHost,
            e: &mut ChatSuggestionAppliedEvent<'_>,
        ) {
            assert_eq!(e.original, "😀 hi st");
            assert_eq!((e.start, e.end), (6, 8));
            assert_eq!(e.input, format!("😀 hi {}", e.suggestion));
            self.0.lock().unwrap().push(e.input.to_owned());
        }
    }
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "listener",
        Box::new(Listener(calls.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        game.screen = Screen::Chat;
        game.ui.registries.players = vec!["Steve".into(), "Steven".into()];
        game.ui.chat_screen = Some(ChatScreen::new(
            &Measure(&game.gui.atlas.font),
            &Commands {
                dispatcher: chat_tree(&game.remote),
                registries: &game.ui.registries,
            },
            "",
            false,
            0,
            640,
            360,
            ChatAbilities::new(ChatVisibility::Full),
        ));
        game.chat_type("😀 hi st");
        game.chat_key_code(KeyCode::Tab, None);
        assert!(calls.lock().unwrap().is_empty(), "only opened the list");
        game.chat_key_code(KeyCode::Tab, None);
        game.chat_key_code(KeyCode::Tab, None);
        assert_eq!(*calls.lock().unwrap(), ["😀 hi Steve", "😀 hi Steven"]);

        // Click the suggestion's rendered text rather than assuming its coordinates.
        let draws = game.ui.chat_screen.as_mut().unwrap().draw(
            &Measure(&game.gui.atlas.font),
            (-1, -1),
            0,
            0,
        );
        let at = draws
            .into_iter()
            .find_map(|d| match d {
                horizon_client::screen_draw::Draw::Text { text, x, y, .. }
                    if text.plain() == "Steve" =>
                {
                    Some(((x + 1) as f32, (y + 1) as f32))
                }
                _ => None,
            })
            .expect("Steve is a visible suggestion");
        game.cursor = at;
        game.chat_click();
        assert_eq!(
            *calls.lock().unwrap(),
            ["😀 hi Steve", "😀 hi Steven", "😀 hi Steve"]
        );
        assert!(
            game.ui
                .chat_screen
                .as_mut()
                .unwrap()
                .take_suggestion_completion()
                .is_none()
        );
        game.chat_key_code(KeyCode::ArrowDown, None);
        assert_eq!(calls.lock().unwrap().len(), 3, "navigation does not apply");
    });
}

/// The chat box open on an empty line, as T opens it.
fn open_chat(game: &mut Game<'_>) {
    game.screen = Screen::Chat;
    game.ui.chat_screen = Some(ChatScreen::new(
        &Measure(&game.gui.atlas.font),
        &Commands {
            dispatcher: chat_tree(&game.remote),
            registries: &game.ui.registries,
        },
        "",
        false,
        0,
        640,
        360,
        ChatAbilities::new(ChatVisibility::Full),
    ));
}

/// `chat_suggestions` through the Game's keys and generated dispatch: asked
/// about plain chat text before the next key, its entries shown by Tab among
/// the players' names and reported by `chat_suggestion_applied` when picked;
/// never asked about a command. With no mod listening nothing is taken.
#[test]
#[ignore = "daily"]
fn plain_chat_suggestions_reach_mods_and_join_the_list() {
    struct Applied(Arc<Mutex<Vec<String>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Applied {
        fn chat_suggestion_applied(
            &self,
            _host: &dyn ClientHost,
            e: &mut ChatSuggestionAppliedEvent<'_>,
        ) {
            self.0.lock().unwrap().push(e.suggestion.to_owned());
        }
    }
    struct Suggester(Arc<Mutex<Vec<(String, usize)>>>, Arc<Mutex<Vec<String>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Suggester {
        fn chat_suggestions(&self, _host: &dyn ClientHost, e: &mut ChatSuggestionsEvent<'_>) {
            self.0.lock().unwrap().push((e.input.to_owned(), e.cursor));
            e.suggestions.push("stack".into());
        }
        fn chat_suggestion_applied(
            &self,
            _host: &dyn ClientHost,
            e: &mut ChatSuggestionAppliedEvent<'_>,
        ) {
            self.1.lock().unwrap().push(e.suggestion.to_owned());
        }
    }
    let mut resources = None;
    let mut game = crate::game::game_tests::tests::server_game(&mut resources);
    game.ui.registries.players = vec!["Steve".into()];

    let applied = Arc::new(Mutex::new(Vec::new()));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "applied",
        Box::new(Applied(applied.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        open_chat(&mut game);
        game.chat_type("hi st");
        game.chat_key_code(KeyCode::Tab, None);
        let screen = game.ui.chat_screen.as_mut().unwrap();
        assert_eq!(
            screen.take_chat_query(),
            Some(("hi st".into(), 5)),
            "no listener: the box's text is not taken"
        );
        game.chat_key_code(KeyCode::Tab, None);
        assert_eq!(*applied.lock().unwrap(), ["Steve"]);
    });

    let asked = Arc::new(Mutex::new(Vec::new()));
    let applied = Arc::new(Mutex::new(Vec::new()));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "suggester",
        Box::new(Suggester(asked.clone(), applied.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        open_chat(&mut game);
        game.chat_type("hi st");
        // Tab shows the list, then picks its entries in turn: "stack" starts
        // with the word as typed, so it comes first.
        for _ in 0..3 {
            game.chat_key_code(KeyCode::Tab, None);
        }
        assert_eq!(*applied.lock().unwrap(), ["stack", "Steve"]);
        assert_eq!(
            *asked.lock().unwrap(),
            [("hi st".to_owned(), 5), ("hi stack".to_owned(), 8)]
        );

        open_chat(&mut game);
        game.chat_type("/gamem");
        game.chat_key_code(KeyCode::Tab, None);
        assert_eq!(asked.lock().unwrap().len(), 2, "a command asks nothing");
    });
}
