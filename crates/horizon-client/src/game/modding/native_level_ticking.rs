//! Ticking eligibility through the same predicates as the server's tick phases.
use super::Game;
use mod_api::LevelRef;

impl Game<'_> {
    fn native_tick_position(&self, level: LevelRef, pos: [i32; 3]) -> Option<bool> {
        if !self.level_in_place(level) {
            return None;
        }
        Some(
            self.level_in_world_bounds(level, pos)?
                && self.chunk(level, [pos[0] >> 4, pos[2] >> 4]).is_some(),
        )
    }

    pub(super) fn level_entity_ticking(&self, level: LevelRef, pos: [i32; 3]) -> Option<bool> {
        Some(self.native_tick_position(level, pos)? && self.entity_ticking_area()(pos[0], pos[2]))
    }

    pub(super) fn level_should_tick_blocks_at(
        &self,
        level: LevelRef,
        pos: [i32; 3],
    ) -> Option<bool> {
        Some(self.native_tick_position(level, pos)? && self.block_ticking_at(pos[0], pos[2]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::native_entity_refs::tests::{prepare_entity_ticking_neighborhood, with_game};
    use horizon_client::ticket_levels::Sources;
    use horizon_core::position::ChunkPos;
    use mod_api::DimensionKey;

    fn assert_reads(game: &Game<'_>, level: LevelRef, pos: [i32; 3], expected: Option<bool>) {
        assert_eq!(level.entity_ticking(game, pos), expected);
        assert_eq!(level.should_tick_blocks_at(game, pos), expected);
    }

    #[test]
    #[ignore = "daily"]
    fn ticking_reads_follow_live_tickets_bounds_and_residency() {
        with_game(|game| {
            let level = game.level("minecraft:overworld").unwrap();
            prepare_entity_ticking_neighborhood(game, ChunkPos::new(-1, -1));
            game.tickets.update(Some(Sources {
                players: vec![(-1, -1)],
                view_distance: 2,
                simulation_distance: 2,
                ..Default::default()
            }));
            let before = game.world.chunks.len();
            for pos in [[-1, 64, -1], [-17, 64, -17], [-1, -64, -1], [-1, 319, -1]] {
                assert_reads(game, level, pos, Some(true));
            }
            // Resident x/z and live tickets isolate the height check.
            for pos in [[-1, -65, -1], [-1, 320, -1], [30_000_000, 64, -1]] {
                assert_reads(game, level, pos, Some(false));
            }
            // A missing center chunk inside the live ticket range.
            game.world.chunks.remove(&(-1, -1)).unwrap();
            assert_reads(game, level, [-1, 64, -1], Some(false));
            assert_eq!(game.world.chunks.len(), before - 1);
            game.world.entities_pending.insert((-2, -2));
            assert_eq!(
                level.should_tick_blocks_at(game, [-17, 64, -17]),
                Some(false)
            );
            game.world.entities_pending.clear();
            assert_reads(game, level, [-17, 64, -17], Some(true));
            game.tickets.update(None);
            assert_reads(game, level, [-17, 64, -17], Some(true));
            game.tickets.update(Some(Sources::default()));
            assert_reads(game, level, [-17, 64, -17], Some(false));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn ticking_reads_refuse_each_unavailable_handle_independently() {
        with_game(|game| {
            let level = game.level("minecraft:overworld").unwrap();
            prepare_entity_ticking_neighborhood(game, ChunkPos::new(0, 0));
            game.tickets.update(None);
            assert_reads(game, level, [0, 64, 0], Some(true));
            let stale = LevelRef::new(
                mod_api::WorldEpoch::new(level.epoch().get() + 1),
                DimensionKey::OVERWORLD,
            );
            assert_reads(game, stale, [0, 64, 0], None);
            let unopened = LevelRef::new(level.epoch(), DimensionKey::NETHER);
            assert_reads(game, unopened, [0, 64, 0], None);
            game.enter_level(crate::game::Dim::Nether).unwrap();
            assert_reads(game, level, [0, 64, 0], None);
        });
        with_game(|game| {
            let level = game.level("minecraft:overworld").unwrap();
            prepare_entity_ticking_neighborhood(game, ChunkPos::new(0, 0));
            game.tickets.update(None);
            assert_reads(game, level, [0, 64, 0], Some(true));
            game.options.execution_role = crate::game::execution_role::ExecutionRole::Client;
            assert_reads(game, level, [0, 64, 0], None);
        });
    }
}
