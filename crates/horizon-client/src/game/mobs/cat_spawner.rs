//! `CatSpawner`: every 1200 ticks a cat may appear near a random player,
//! in a village with enough occupied beds or inside a swamp hut.
use super::Game;

pub(super) use horizon_gameplay::cat_spawner::CatSpawner;

impl Game<'_> {
    /// `CatSpawner.tick` (`horizon_gameplay::cat_spawner`) on the game level.
    pub(super) fn tick_cat_spawner(&mut self) {
        let mut cat = std::mem::take(&mut self.creatures.cat);
        cat.tick(self);
        self.creatures.cat = cat;
    }
}

#[cfg(test)]
#[path = "tests/cat_spawner_tests.rs"]
mod cat_spawner_tests;
