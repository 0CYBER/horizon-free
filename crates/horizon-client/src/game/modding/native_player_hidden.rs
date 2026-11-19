//! `player_hidden`: reads back what `player_hide` set.
use super::Game;
use mod_api::PlayerRef;

impl Game<'_> {
    /// The level `player_hide` hides the player from, None when shown. The outer None is a
    /// stale world epoch or a client.
    pub(super) fn player_hidden(&self, player: PlayerRef) -> Option<Option<u8>> {
        if self.native_lifecycle.epoch() != Some(player.entity().epoch()) {
            return None;
        }
        let host = self.lan.as_ref()?;
        Some(host.hub.hidden.get(&player.uuid()).copied())
    }
}

#[cfg(test)]
mod tests {
    use super::super::native_guest_turn_tests::with_guests;

    /// What `player_hide` sets reads back, by UUID, until it is shown again.
    #[test]
    #[ignore = "daily"]
    fn player_hidden_reads_back_the_hide_level() {
        with_guests(|game| {
            let a = game.player(12).unwrap();
            let b = game.player(13).unwrap();
            assert_eq!(game.player_hidden(a), Some(None));
            assert!(game.player_hide(a, Some(2)));
            assert_eq!(game.player_hidden(a), Some(Some(2)));
            assert_eq!(game.player_hidden(b), Some(None));
            assert!(game.player_hide(a, None));
            assert_eq!(game.player_hidden(a), Some(None));
            let stale = mod_api::__private::game::player_ref(
                mod_api::EntityRef::new(
                    mod_api::WorldEpoch::new(a.entity().epoch().get() + 1),
                    a.entity().id(),
                ),
                a.uuid(),
            );
            assert_eq!(game.player_hidden(stale), None);
        });
    }
}
