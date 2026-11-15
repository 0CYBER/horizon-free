/// This mod's counters' block entity type.
const OWN: &str = "fnr_loop:counter";

/// Counts the visits and ticks it sees, and this mod's counters among them, and always answers
/// Continue. On its 500th visit it runs the whole loop again from inside it: the nested run is the
/// game's own, without the slots.
pub struct Count {
    own: OwnBlockEntities,
    visited: u64,
    counters: u64,
    ticked: u64,
}

impl Default for Count {
    fn default() -> Self {
        Self {
            own: OwnBlockEntities::new(&[OWN]),
            visited: 0,
            counters: 0,
            ticked: 0,
        }
    }
}

impl BlockEntityTickSlot for Count {
    fn before(&mut self, tick: &mut BlockEntityTick<'_, '_>) -> LoopFlow {
        self.visited += 1;
        if self.visited % 1000 == 1 {
            let cooldown = tick.game.hoppers.get(&tick.pos).map(|state| state.cooldown);
            fnr_loop::note(&format!(
                "count before #{}: {:?} at {:?} in {:?}, time {}, cooldown {cooldown:?}",
                self.visited,
                tick.kind,
                tick.pos,
                tick.game.dim,
                tick.game.world.game_time,
            ));
        }
        if self.own.contains(tick) {
            self.counters += 1;
            // The check, on every recognition: the slow way, the entity's id must be this mod's.
            let pos = tick.pos;
            if let Some(BlockEntity::Mod(entity)) = tick.block_entity() {
                if entity.id() != OWN {
                    fnr_loop::note(&format!("count own FOREIGN at {pos:?}: {}", entity.id()));
                } else if self.counters % 200 == 1 {
                    fnr_loop::note(&format!("count own #{}: {}", self.counters, entity.id()));
                }
            }
        }
        if self.visited == 500 {
            let nested = tick.game.tick_block_entities();
            fnr_loop::note(&format!("count ran the loop again inside it: {}", nested.is_ok()));
        }
        LoopFlow::Continue
    }
    fn after(&mut self, tick: &mut BlockEntityTick<'_, '_>) {
        self.ticked += 1;
        if self.ticked % 1000 == 1 {
            fnr_loop::note(&format!("count after #{}: {:?}", self.ticked, tick.kind));
        }
    }
}
