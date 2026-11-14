//! A block a mod writes as a player placed it: `BlockItem.place`'s write from a chosen state.
use super::{Game, native_server::PlayerRead};
use horizon_core::position::BlockPos;
use horizon_world::{level::PlaceStateRefusal, registry::BlockStateId};
use mod_api::{LevelRef, PlayerRef, PlayerWorldRefusal as Refusal};

impl Game<'_> {
    /// `Behaviours::place_block_state` in the player's turn, then the block entity a new block
    /// brings, as a placement makes it; the same block in another state keeps its own.
    pub(super) fn player_place_block(
        &mut self,
        player: PlayerRef,
        level: LevelRef,
        pos: [i32; 3],
        state: BlockStateId,
        update_neighbour_shapes: bool,
    ) -> Result<(), Refusal> {
        if !self.is_logical_server() {
            return Err(Refusal::Client);
        }
        let dead = match self.player_handle_read(player).ok_or(Refusal::Stale)? {
            PlayerRead::Acting { .. } => self
                .player
                .as_ref()
                .is_none_or(|p| p.health.health() <= 0.0),
            PlayerRead::Guest(g) => g.dead,
        };
        if dead {
            return Err(Refusal::Dead);
        }
        if self.pack.assets.registry.state(state).is_none() {
            return Err(Refusal::UnknownState);
        }
        if self.native_level_world(level).is_none() {
            return Err(Refusal::StaleLevel);
        }
        if level.dimension() != self.dim.key() {
            return Err(Refusal::OtherLevel);
        }
        let mut written = Err(Refusal::Stale);
        let ran = self.native_as_player_handle(player, |game| {
            let previous = game.world.block(pos[0], pos[1], pos[2]);
            let at = BlockPos::new(pos[0], pos[1], pos[2]);
            let outcome = game.with_level(|b, level, ticks| {
                b.place_block_state(level, ticks, at, state, update_neighbour_shapes)
            });
            let now = match outcome {
                Ok(now) => now,
                Err(refusal) => {
                    let _ = game.process_level_events();
                    written = Err(match refusal {
                        PlaceStateRefusal::OutOfWorld => Refusal::OutOfWorld { pos },
                        PlaceStateRefusal::NotLoaded => Refusal::NotLoaded { pos },
                        PlaceStateRefusal::Unchanged => Refusal::Unchanged,
                    });
                    return;
                }
            };
            let key = (pos[0], pos[1], pos[2]);
            let facts = game.res.facts();
            let same_block = facts.get(previous).name == facts.get(now).name;
            if !same_block
                && !game.world.block_entities.stateful_entity(now.0)
                && let Some(entity) = game.new_block_entity_for_state(now)
            {
                game.world.block_entities.insert(key, entity);
            }
            let _ = game.process_level_events();
            written = Ok(());
        });
        if ran { written } else { Err(Refusal::Stale) }
    }
}
