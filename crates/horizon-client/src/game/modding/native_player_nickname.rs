//! `player_nickname`: reads back what `player_set_nickname` set.
use super::Game;
use mod_api::PlayerRef;

impl Game<'_> {
    /// The player's nickname, None when they have none; the outer None is a stale or
    /// offline player, or no server list.
    pub(super) fn player_nickname(&self, player: PlayerRef) -> Option<Option<String>> {
        let online = self.native_acting_player() == Some(player)
            || self.online_player_record(player).is_some();
        if !online {
            return None;
        }
        let host = self.lan.as_ref()?;
        Some(host.hub.nicknames.get(&player.uuid()).cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::super::native_guest_turn_tests::with_guests;

    /// What `player_set_nickname` sets reads back for that player only.
    #[test]
    #[ignore = "daily"]
    fn player_nickname_reads_back_the_set_nickname() {
        with_guests(|game| {
            let a = game.player(12).unwrap();
            let b = game.player(13).unwrap();
            assert_eq!(game.player_nickname(a), Some(None));
            assert!(game.player_set_nickname(a, Some("Captain")));
            assert_eq!(game.player_nickname(a), Some(Some("Captain".to_owned())));
            assert_eq!(game.player_nickname(b), Some(None));
            assert!(game.player_set_nickname(a, None));
            assert_eq!(game.player_nickname(a), Some(None));
            let gone = mod_api::__private::game::player_ref(a.entity(), 999);
            assert_eq!(game.player_nickname(gone), None);
        });
    }
}
