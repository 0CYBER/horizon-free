//! `Hud.extractRenderState`'s subtitle rule and `Screen.isInGameUi`.
use super::Screen;

/// `Screen.isInGameUi`: the screens that keep the game behind them — the
/// container screens (`AbstractContainerScreen`) and the block editors
/// (`AbstractCommandBlockEditScreen`, structure block, jigsaw, test block) —
/// plus the two Horizon draws over `Screen::Playing`: a book (`BookViewScreen`)
/// and a sign editor (`AbstractSignEditScreen`).
pub(super) fn in_game_ui(screen: Screen, book_or_sign: bool) -> bool {
    book_or_sign
        || matches!(
            screen,
            Screen::Inventory | Screen::Creative | Screen::CommandBlock | Screen::BlockEditor
        )
}

/// Where `Hud.extractRenderState` draws the subtitles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Subtitles {
    /// Hidden with the rest of the HUD: F1 with no screen over the game.
    Hidden,
    /// In the HUD's own pass, under any screen that draws later.
    Now,
    /// Deferred (`Hud.extractDeferredSubtitles`): an in-game screen draws
    /// them from `Screen.extractBackground`, under its slots and tooltips,
    /// whether or not the HUD is hidden. Horizon draws them just before the
    /// screen, which keeps that order (only the screen's dimming goes over them).
    Deferred,
}

/// `Hud.extractRenderState`'s subtitle rule: with no in-game screen the
/// subtitles follow the HUD (`!isHidden`); with one they are deferred and
/// draw over it, which is the one HUD element F1 does not take away.
pub(super) fn subtitles(show_hud: bool, in_game_ui: bool) -> Subtitles {
    if in_game_ui {
        Subtitles::Deferred
    } else if show_hud {
        Subtitles::Now
    } else {
        Subtitles::Hidden
    }
}

#[cfg(test)]
mod tests {
    /// `Hud.extractRenderState`: the subtitles follow the HUD when no screen
    /// is over the game, are deferred over a screen that keeps the game
    /// behind it (so F1 does not take them away), and vanish under F1 alone.
    #[test]
    fn the_subtitles_follow_the_hud_unless_a_screen_keeps_them() {
        use super::{Screen, Subtitles, in_game_ui, subtitles};
        // A container screen defers them above itself, hidden HUD or not.
        assert_eq!(subtitles(true, true), Subtitles::Deferred);
        assert_eq!(subtitles(false, true), Subtitles::Deferred);
        // No such screen: in the HUD's pass, and gone under F1.
        assert_eq!(subtitles(true, false), Subtitles::Now);
        assert_eq!(subtitles(false, false), Subtitles::Hidden);
        // `Screen.isInGameUi` is `AbstractContainerScreen` and the block
        // editors' answer; a pause menu, the chat and the death screen say no.
        assert!(in_game_ui(Screen::Inventory, false));
        assert!(in_game_ui(Screen::Creative, false));
        assert!(in_game_ui(Screen::CommandBlock, false));
        assert!(in_game_ui(Screen::BlockEditor, false));
        // A book and a sign editor are in-game UI drawn over `Playing`.
        assert!(in_game_ui(Screen::Playing, true));
        assert!(!in_game_ui(Screen::Paused, false));
        assert!(!in_game_ui(Screen::Chat, false));
        assert!(!in_game_ui(Screen::Dead, false));
        assert!(!in_game_ui(Screen::Playing, false));
    }
}
