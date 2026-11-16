/// This mod's counters' block entity type.
const OWN: &str = "fnr_loop_second:counter";

/// Counts this mod's counters among the block entities the loop visits, and always answers Continue.
pub struct Own {
    own: OwnBlockEntities,
    counted: u64,
}

impl Default for Own {
    fn default() -> Self {
        Self {
            own: OwnBlockEntities::new(&[OWN]),
            counted: 0,
        }
    }
}

impl BlockEntityTickSlot for Own {
    fn before(&mut self, tick: &mut BlockEntityTick<'_, '_>) -> LoopFlow {
        if self.own.contains(tick) {
            self.counted += 1;
            // The check, on every recognition: the slow way, the entity's id must be this mod's.
            let pos = tick.pos;
            if let Some(BlockEntity::Mod(entity)) = tick.block_entity() {
                if entity.id() != OWN {
                    fnr_loop_second::note(&format!("own FOREIGN at {pos:?}: {}", entity.id()));
                } else if self.counted % 200 == 1 {
                    fnr_loop_second::note(&format!("own #{}: {}", self.counted, entity.id()));
                }
            }
        }
        LoopFlow::Continue
    }
}
