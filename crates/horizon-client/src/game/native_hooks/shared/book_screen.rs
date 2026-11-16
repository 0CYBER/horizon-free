use super::*;
#[path = "book_screen/open.rs"]
mod open;
#[path = "book_screen/placement.rs"]
mod placement;

#[cfg(test)]
#[path = "book_screen/lifecycle_tests.rs"]
mod lifecycle_tests;

impl Game<'_> {
    /// The mod screen showing now, if any.
    pub(in crate::game) fn shown_native_screen(&self) -> Option<&OpenNativeScreen> {
        self.mod_screen
            .as_ref()
            .filter(|s| self.screen == Screen::Inventory && self.container == Some(s.key))
    }
    /// Whether a mod's screen places the current container and player slots.
    pub(in crate::game) fn native_screen_shown(&self) -> bool {
        self.shown_native_screen().is_some()
    }
    /// The open mod screen's height.
    pub(in crate::game) fn native_screen_height(&self) -> Option<f32> {
        self.shown_native_screen().map(|s| s.screen.height)
    }
    pub(in crate::game) fn native_screen_menu(&self) -> Option<&str> {
        let screen = self.shown_native_screen()?;
        self.remote_menu_type()
            .or_else(|| self.mod_menu_at(screen.key))
    }
    pub(in crate::game) fn native_screen_toggle(&self) -> Option<[f32; 2]> {
        self.shown_native_screen()?.screen.recipe_book_toggle
    }
    pub(in crate::game) fn native_screen_block(&self) -> Option<&str> {
        Some(&self.shown_native_screen()?.block)
    }
    /// The explicitly assigned book; declarations bind it to the actual menu.
    pub(in crate::game) fn native_screen_book(&self) -> Option<&mod_api::RecipeBookTypeKey> {
        self.shown_native_screen()?.screen.recipe_book.as_ref()
    }
}
