//! Read-only chunk facts that never request residency.
use super::Game;
use horizon_core::random::{RandomSource, slime_chunk_random};
use mod_api::ChunkRef;

impl Game<'_> {
    pub(super) fn chunk_is_slime_chunk(&self, chunk: ChunkRef) -> Option<bool> {
        let seed = self.level_seed(chunk.level())?;
        let [x, z] = chunk.pos();
        // Reuse the same world-bound validation as handles for unloaded chunks.
        self.level_chunk_ref(chunk.level(), [x, z])?;
        Some(
            slime_chunk_random(x, z, seed, 987_234_911)
                .next_int_bound(10)
                .unwrap_or(1)
                == 0,
        )
    }
}

#[cfg(test)]
mod tests {
    use crate::game::native_entity_refs::tests::with_game;
    use mod_api::{Authority, ChunkRef, DimensionKey, LevelRef};

    #[test]
    #[ignore = "daily"]
    fn slime_queries_work_without_loading_and_refuse_stale_handles() {
        with_game(|game| {
            game.level.seed = 0;
            let level = game.level("minecraft:overworld").unwrap();
            let before = game.world.chunks.len();
            let far = (100_000, -100_000);
            assert!(!game.world.chunks.contains_key(&far));
            // Independent Java LCG calculation for seed 0: the first nextInt(10)
            // draws are 7, 1 and 0 respectively. Int terms wrap before widening;
            // the 48-bit LCG uses multiplier 0x5deece66d and addend 11.
            for (pos, expected) in [
                ([0, 0], false),
                ([-17, 29], false),
                ([100_000, -100_000], true),
            ] {
                let chunk = level.chunk_ref(game, pos).unwrap();
                assert_eq!(chunk.is_slime_chunk(game), Some(expected));
                assert_eq!(chunk.is_slime_chunk(game), Some(expected));
            }
            assert_eq!(game.world.chunks.len(), before);
            assert!(!game.world.chunks.contains_key(&far));
            let invalid = ChunkRef::new(level, [i32::MAX, 0]);
            assert_eq!(invalid.is_slime_chunk(game), None);
            let unopened = LevelRef::new(level.epoch(), DimensionKey::NETHER);
            assert_eq!(ChunkRef::new(unopened, [0, 0]).is_slime_chunk(game), None);
            game.enter_level(crate::game::Dim::Nether).unwrap();
            let nether = game.level("minecraft:the_nether").unwrap();
            game.level.seed = 0;
            let parked = ChunkRef::new(level, [-17, 29]);
            assert_eq!(parked.is_slime_chunk(game), Some(false));
            let active = ChunkRef::new(nether, [100_000, -100_000]);
            assert!(!game.world.chunks.contains_key(&far));
            assert_eq!(active.is_slime_chunk(game), Some(true));
            assert!(!game.world.chunks.contains_key(&far));
            // These handles are still current: refusal must be due to client role.
            game.options.execution_role = crate::game::execution_role::ExecutionRole::Client;
            assert_eq!(active.is_slime_chunk(game), None);
            assert_eq!(parked.is_slime_chunk(game), None);
            game.options.execution_role =
                crate::game::execution_role::ExecutionRole::IntegratedServer;
            game.native_lifecycle
                .begin_open(Authority::IntegratedServer);
            assert_eq!(ChunkRef::new(level, [0, 0]).is_slime_chunk(game), None);
            assert_eq!(active.is_slime_chunk(game), None);
        });
    }
}
