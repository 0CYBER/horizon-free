//! Player-owned custom recipe book settings, separate from vanilla ordinals.
use super::Game;
use mod_api::{
    PlayerRef,
    item::recipe_book::{RecipeBookSettings, RecipeBookTypeKey},
};

impl Game<'_> {
    pub(super) fn player_synced_recipe_book_settings(
        &self,
        player: PlayerRef,
        book: &RecipeBookTypeKey,
    ) -> Option<RecipeBookSettings> {
        self.native_local_status(player)?;
        self.custom_book_settings(book)
    }
    pub(super) fn player_request_recipe_book_settings(
        &mut self,
        player: PlayerRef,
        book: &RecipeBookTypeKey,
        settings: RecipeBookSettings,
    ) -> bool {
        self.native_local_status(player).is_some()
            && self.request_custom_book_settings(book, settings)
    }

    pub(super) fn player_set_recipe_book_settings(
        &mut self,
        player: PlayerRef,
        book: &RecipeBookTypeKey,
        settings: RecipeBookSettings,
    ) -> bool {
        if !book.is_registered() || !self.native_player_current(player) {
            return false;
        }
        let flags = horizon_client::recipe_book::BookFlags {
            open: settings.open,
            filtering: settings.filtering,
        };
        let mut result = Err(horizon_client::recipe_book::BookError::Full);
        let ran = self.native_as_player_handle(player, |game| {
            result = game.level.recipe_book.custom.set(book.as_str(), flags);
        });
        if !ran {
            return false;
        }
        let Ok(changed) = result else {
            return false;
        };
        if changed {
            self.sync_server_book_write(player, book, flags);
            if let Some(name) = self.player_name(player) {
                self.native_player_recipe_book_settings_change(
                    player.uuid(),
                    &name,
                    mod_api::RecipeBookIdentity::Custom(book.clone()),
                    flags.open,
                    flags.filtering,
                );
            }
        }
        true
    }

    pub(super) fn player_recipe_book_settings(
        &self,
        player: PlayerRef,
        book: &RecipeBookTypeKey,
    ) -> Option<RecipeBookSettings> {
        if !book.is_registered() || !self.native_player_current(player) {
            return None;
        }
        let settings = match self.native_player_resting(player) {
            Some(guest) => &guest.recipe_book,
            None => &self.level.recipe_book,
        };
        let flags = settings.custom.get(book.as_str());
        Some(RecipeBookSettings {
            open: flags.open,
            filtering: flags.filtering,
        })
    }
}
