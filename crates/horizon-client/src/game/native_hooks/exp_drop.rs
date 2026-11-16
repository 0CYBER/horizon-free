use super::*;
impl GameLevel<'_> {
    pub(in crate::game) fn native_exp_drop(&self, call: super::fire::ExpDropCall<'_>) {
        self.fire_exp_drop(call);
    }
}

impl GameLevel<'_> {
    pub(in crate::game) fn part_break_experience(
        &self,
        pos: BlockPos,
        state: BlockStateId,
        breaker: Option<u128>,
        tool: Option<&ItemStack>,
    ) -> Option<u32> {
        if !self.native_authoritative
            || !crate::game::api_values::block_state::block_listens(mod_api::HookId::ExpDrop, state)
        {
            return None;
        }
        let (block_entity, breaker) = self.drop_event_refs(pos, breaker);
        let mut experience = None;
        self.native_exp_drop(super::fire::ExpDropCall {
            dimension: self.dimension,
            pos,
            state,
            block_entity,
            breaker,
            tool,
            experience: &mut experience,
        });
        experience.map(|value| value.clamp(0, 32767) as u32)
    }
}

impl GameLevel<'_> {
    /// The block entity at `pos` and the entity that broke it, as a block's drop and experience
    /// hooks give them (`BlockDropsEvent.getBlockEntity` and `getBreaker`): the block entity with
    /// its generation, the player's entity for a player's break (`breaker`, their UUID), or the
    /// mob in `drop_entity` while its break at this position runs.
    pub(in crate::game) fn drop_event_refs(
        &self,
        pos: BlockPos,
        breaker: Option<u128>,
    ) -> (Option<mod_api::BlockEntityRef>, Option<mod_api::EntityRef>) {
        let at = [pos.x, pos.y, pos.z];
        let block_entity = self.api_level.and_then(|level| {
            // The one a player's break just removed stays held for its `block_drops` event.
            let generation = if self
                .world
                .block_entities
                .get(&(pos.x, pos.y, pos.z))
                .is_some()
            {
                self.world
                    .block_entities
                    .generation(&(pos.x, pos.y, pos.z))?
            } else {
                self.world
                    .block_entities
                    .broken_generation(&(pos.x, pos.y, pos.z))?
            };
            Some(mod_api::__private::game::block_entity_ref(
                level, at, generation,
            ))
        });
        let breaker = if breaker.is_some() {
            self.api_level
                .zip(self.native_player_entity)
                .map(|(level, id)| mod_api::EntityRef::new(level.epoch(), id as u32))
        } else {
            self.drop_entity
                .filter(|_| self.drop_breaker_at == Some(pos))
        };
        (block_entity, breaker)
    }
}

impl Game<'_> {
    pub(in crate::game) fn prepare_part_break_experience(
        &mut self,
        pos: [i32; 3],
        state: BlockStateId,
        experience: &mut Option<u32>,
    ) {
        if !self.native_authoritative()
            || !crate::game::api_values::block_state::block_listens(mod_api::HookId::ExpDrop, state)
        {
            return;
        }
        let tool = self.held().cloned();
        let breaker = Some(self.native_acting_id());
        *experience = self.with_level(|_, level, _| {
            let pos = BlockPos::new(pos[0], pos[1], pos[2]);
            let amount = level
                .part_break_experience(pos, state, breaker, tool.as_ref())
                .unwrap_or_else(|| {
                    horizon_world::level::removal::break_experience(
                        &mut level.world.random,
                        &level.facts.get(state).name,
                    )
                    .unwrap_or(0)
                });
            Some(
                if tool
                    .as_ref()
                    .is_some_and(|tool| tool.enchantment_level("minecraft:silk_touch") > 0)
                    && &*level.facts.get(state).name != "minecraft:spawner"
                {
                    0
                } else {
                    amount
                },
            )
        });
    }
}

impl Game<'_> {
    pub(in crate::game) fn entity_block_dropped(
        &mut self,
        pos: [i32; 3],
        state: BlockStateId,
        breaker: Option<u32>,
        drops: Vec<ItemStack>,
    ) -> crate::game::spill::BlockDrops {
        let breaker =
            if crate::game::api_values::block_state::block_listens(mod_api::HookId::ExpDrop, state)
                || mod_api::__private::listened(mod_api::HookId::BlockDrops)
            {
                breaker.and_then(|id| self.entity(id))
            } else {
                None
            };
        self.with_level(|_, level, _| {
            let pos = BlockPos::new(pos[0], pos[1], pos[2]);
            // with_level rebuilds GameLevel with both fields None on every call.
            level.drop_entity = breaker;
            level.drop_breaker_at = Some(pos);
            level.block_dropped(pos, state, None, None, drops, None)
        })
    }
}

/// A player's break, before the removal: the block entity (for what the drops copy from it),
/// and for the `block_drops` event its identity and saved data with the `CustomName`, which the
/// removal clears from the level.
pub(in crate::game) type LiveBlockEntity = (
    horizon_client::containers::BlockEntity,
    Option<(Compound, u64)>,
);

impl Game<'_> {
    /// The block entity at `key` as a player's break finds it before the removal takes it out of
    /// the world. The saved data is read only when a `block_drops` listener will want it.
    pub(in crate::game) fn live_block_entity(
        &self,
        key: (i32, i32, i32),
        block: &str,
    ) -> Option<LiveBlockEntity> {
        let entity = self.world.block_entities.get(&key)?.clone();
        let view = (self.native_authoritative()
            && mod_api::__private::listened(mod_api::HookId::BlockDrops))
        .then(|| {
            let data = self.saved_block_entity_data(key, &entity, block)?;
            Some((data, self.world.block_entities.generation(&key)?))
        })
        .flatten();
        Some((entity, view))
    }
    /// `Block.dropResources` for a player's break (`block_dropped`), with the data of the block
    /// entity the removal took (`live`, from [`Self::live_block_entity`]) held readable for the
    /// `block_drops` event (`BlockDropsEvent.getBlockEntity`) and released after it, also when a
    /// listener panics: handles to it are stale from then on. `at` is the position and the state
    /// after the removal; `who` the breaker's UUID, their tool and the held block entity; `loot`
    /// the stacks and an experience override.
    pub(in crate::game) fn player_block_dropped(
        &mut self,
        at: ([i32; 3], BlockStateId),
        who: (Option<u128>, Option<&ItemStack>, Option<LiveBlockEntity>),
        loot: (Vec<ItemStack>, Option<u32>),
    ) -> crate::game::spill::BlockDrops {
        let ([x, y, z], state) = at;
        let (breaker, tool, live) = who;
        if let Some((data, generation)) = live.and_then(|(_, view)| view) {
            self.world
                .block_entities
                .hold_broken((x, y, z), generation, data);
        }
        let dropped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.with_level(|_, level, _| {
                level.block_dropped(BlockPos::new(x, y, z), state, breaker, tool, loot.0, loot.1)
            })
        }));
        self.world.block_entities.release_broken();
        dropped.unwrap_or_else(|panic| std::panic::resume_unwind(panic))
    }
}
