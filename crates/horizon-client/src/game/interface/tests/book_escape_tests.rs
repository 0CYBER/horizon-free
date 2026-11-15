//! Escape on the two book screens, and Enter on the sign screen's title.
//!
//! `BookEditScreen` and `BookSignScreen` override neither `onClose` nor
//! `shouldCloseOnEsc`, so Escape is `Screen.onClose` (`setScreen(null)`,
//! `Screen.java:121-125,197-203`) and saves nothing: only the Done button
//! calls `BookEditScreen.saveChanges`. `BookSignScreen.keyPressed`
//! (`BookSignScreen.java:79-87`) signs on Enter whenever the title box is not
//! empty — `!getValue().isEmpty()`, not blank — and trims the title on send
//! (`:70`).
use super::{BookScreen, Game};
use crate::game::game_tests::tests::server_game;
use horizon_client::items::ItemStack;
use winit::keyboard::KeyCode;

/// A game whose selected hotbar slot holds a book and quill, its editor open,
/// `text` typed on the page.
fn editing(text: &str) -> Game<'static> {
    let resources = Box::leak(Box::new(None));
    let mut game = server_game(resources);
    *game.held_mut() = Some(ItemStack::new("minecraft:writable_book", 1));
    let stack = game.held().expect("a book in hand").clone();
    assert!(game.remote_use_book(&stack), "the editor opened");
    game.book_input(None, Some(text));
    game
}

/// The pages saved into the stack in hand (empty: nothing was written to it).
fn saved_pages(game: &Game<'_>) -> Vec<String> {
    game.held()
        .and_then(|stack| stack.data())
        .map(|data| data.writable_pages.clone())
        .unwrap_or_default()
}

/// `BookEditScreen` has no `onClose`: Escape is `Screen.onClose`, which closes
/// the screen and sends nothing, so the typed page is lost.
#[test]
#[ignore = "daily"]
fn escape_in_the_editor_saves_nothing() {
    let mut game = editing("hello");
    game.book_input(Some(KeyCode::Escape), None);
    assert!(game.book.is_none(), "Escape left the editor open");
    assert!(saved_pages(&game).is_empty(), "Escape saved the page");
}

/// `BookSignScreen` has no `onClose` either: Escape closes the whole book
/// rather than going back to the editor, and saves nothing.
#[test]
#[ignore = "daily"]
fn escape_on_the_sign_screen_closes_the_book() {
    let mut game = editing("hi");
    game.book_control(Some("sign")).expect("the Sign button");
    assert!(matches!(
        game.book,
        Some(BookScreen::Edit { signing: true, .. })
    ));
    game.book_input(Some(KeyCode::Escape), None);
    assert!(
        game.book.is_none(),
        "Escape went back to the editor instead of closing the book"
    );
    assert!(saved_pages(&game).is_empty(), "Escape saved the page");
}

/// `BookSignScreen.keyPressed` signs on Enter for any title that is not empty,
/// so three spaces sign a book whose title trims to nothing; an empty title
/// stays unsignable.
#[test]
#[ignore = "daily"]
fn enter_signs_a_title_of_spaces_but_not_an_empty_one() {
    let mut game = editing("hi");
    game.book_control(Some("sign")).expect("the Sign button");
    game.book_input(Some(KeyCode::Enter), None);
    assert!(
        game.book.is_some(),
        "Enter signed a book with an empty title"
    );
    game.book_input(None, Some("   "));
    game.book_input(Some(KeyCode::Enter), None);
    let held = game.held().expect("a book in hand");
    assert_eq!(
        &*held.item, "minecraft:written_book",
        "Enter ignored a title of spaces"
    );
    let title = held
        .data()
        .and_then(|data| data.written.as_ref())
        .map(|book| book.title.clone());
    assert_eq!(title.as_deref(), Some(""), "the title is trimmed on send");
}
