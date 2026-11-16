//! Client crack state belongs to one level and expires independently of new updates.
use super::Game;

impl Game<'_> {
    pub(super) fn expire_client_cracks(&mut self) {
        let now = self.world.game_time;
        self.destroying
            .retain(|_, crack| now.saturating_sub(crack.2) <= 400);
    }

    pub(super) fn reset_client_world(&mut self, seed: i64) {
        self.destroying.clear();
        self.world = Game::world_for_mods(self.res.facts().air, seed);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "daily"]
    fn client_cracks_clear_expire_and_do_not_cross_worlds() {
        super::super::native_entity_refs::tests::with_client_game(|game| {
            game.destroy_block_progress(-10, [1, 64, 1], 4);
            game.destroy_block_progress(-11, [2, 64, 1], 9);
            assert_eq!(game.destroying.len(), 2);
            game.destroy_block_progress(-10, [1, 64, 1], -1);
            assert!(!game.destroying.contains_key(&-10));
            assert!(game.destroying.contains_key(&-11));
            game.world.game_time += 401;
            game.tick().unwrap();
            assert!(game.destroying.is_empty());
            game.destroy_block_progress(-11, [2, 64, 1], 3);
            game.reset_client_world(0);
            assert!(game.destroying.is_empty());
        });
    }
}
