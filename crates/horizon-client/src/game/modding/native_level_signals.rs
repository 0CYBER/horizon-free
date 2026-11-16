//! Shared level input for the canonical signal evaluator and its block Parts.
use super::Game;
use super::api_values::{TryIntoApi, TryIntoEngine};
use horizon_client::{items::Items, world::World};
use horizon_core::{geometry::Direction, position::BlockPos};
use horizon_world::{level::signal_access::SignalAccess, registry::BlockStateId};
use mod_api::LevelHost;
use std::collections::HashMap;

/// Created only after `level_in_place` accepts the level. No mutable world,
/// random source, entity snapshots or transaction is needed by a signal read.
pub(super) struct ReadLevel<'g, 'a> {
    pub game: &'g Game<'a>,
    pub level: mod_api::LevelRef,
}

super::native_tags::impl_tags!(ReadLevel<'_, '_>, level, level.game.native_tag_items());

/// The output side passed to Parts is opposite SignalGetter's input direction.
/// Both the shared view and the write context apply the same output bound.
pub(super) fn adjust_signal(
    direction: Direction,
    vanilla: i32,
    fire: impl FnOnce(Direction, &mut i32),
) -> i32 {
    let mut signal = vanilla;
    fire(direction.opposite(), &mut signal);
    signal.clamp(0, 15)
}

/// Block-entity-driven sources, shared by the read and write views.
pub(super) fn block_entity_signal(
    world: &World,
    registry: &horizon_world::registry::BlockRegistry,
    items: &Items,
    lid_openers: &HashMap<(i32, i32, i32), i32>,
    pos: BlockPos,
    state: BlockStateId,
) -> i32 {
    if let Some(horizon_client::containers::BlockEntity::TestBlock(block)) =
        world.block_entities.get(&(pos.x, pos.y, pos.z))
    {
        return super::test_blocks::own_signal(registry, state, block.powered);
    }
    if let Some(count) = lid_openers.get(&(pos.x, pos.y, pos.z)) {
        return (*count).clamp(0, 15);
    }
    super::jukebox::jukebox_signals(world, items, pos).map_or(0, |(signal, _)| signal)
}

impl SignalAccess for ReadLevel<'_, '_> {
    fn block_state(&self, pos: BlockPos) -> BlockStateId {
        self.game.world.block(pos.x, pos.y, pos.z)
    }
    fn shulker_is_closed(&self, pos: BlockPos) -> Option<bool> {
        self.game
            .be_anims
            .shulkers
            .get(&[pos.x, pos.y, pos.z])
            .map(|s| s.status == horizon_client::block_entity_anim::ShulkerStatus::Closed)
    }
    fn adjust_signal_source(&self, state: BlockStateId, vanilla: bool) -> bool {
        if !super::api_values::block_state::block_listens(mod_api::HookId::IsSignalSource, state) {
            return vanilla;
        }
        use super::api_values::TryIntoApi;
        let Ok(state) = state.try_into_api() else {
            return vanilla;
        };
        let mut source = vanilla;
        mod_api::dispatch_is_signal_source(self, state, &mut source);
        source
    }
    fn has_weak_power_hook(&self) -> bool {
        mod_api::__private::has_scoped_hook(mod_api::HookId::ShouldCheckWeakPower)
    }
    fn adjust_should_check_weak_power(
        &self,
        pos: BlockPos,
        state: BlockStateId,
        direction: Direction,
        vanilla: bool,
    ) -> bool {
        if !super::api_values::block_state::block_listens(
            mod_api::HookId::ShouldCheckWeakPower,
            state,
        ) {
            return vanilla;
        }
        use super::api_values::TryIntoApi;
        let Ok(state) = state.try_into_api() else {
            return vanilla;
        };
        let mut check = vanilla;
        mod_api::dispatch_should_check_weak_power(
            self,
            self.game.dim.level(),
            pos,
            state,
            direction,
            &mut check,
        );
        check
    }
    fn has_redstone_wire_connection_hook(&self) -> bool {
        mod_api::__private::has_scoped_hook(mod_api::HookId::ShouldRedstoneWireConnectTo)
    }
    fn adjust_should_redstone_wire_connect_to(
        &self,
        pos: BlockPos,
        state: BlockStateId,
        direction: Option<Direction>,
        vanilla: bool,
    ) -> bool {
        if !super::api_values::block_state::block_listens(
            mod_api::HookId::ShouldRedstoneWireConnectTo,
            state,
        ) {
            return vanilla;
        }
        use super::api_values::TryIntoApi;
        let Ok(state) = state.try_into_api() else {
            return vanilla;
        };
        let mut connected = vanilla;
        mod_api::dispatch_should_redstone_wire_connect_to(
            self,
            self.game.dim.level(),
            pos,
            state,
            direction,
            &mut connected,
        );
        connected
    }
    fn adjust_signal(
        &self,
        pos: BlockPos,
        state: BlockStateId,
        direction: Direction,
        direct: bool,
        vanilla: i32,
    ) -> i32 {
        if !super::api_values::block_state::block_listens(mod_api::HookId::BlockSignal, state) {
            return vanilla;
        }
        use super::api_values::TryIntoApi;
        let Ok(state) = state.try_into_api() else {
            return vanilla;
        };
        adjust_signal(direction, vanilla, |face, signal| {
            mod_api::dispatch_block_signal(
                self,
                self.game.dim.level(),
                pos,
                state,
                face,
                direct,
                signal,
            );
        })
    }
    fn block_entity_signal(&self, pos: BlockPos, state: BlockStateId, _: Direction) -> i32 {
        block_entity_signal(
            &self.game.world,
            self.game.pack.assets.registry,
            self.game.items(),
            &self.game.lid_openers,
            pos,
            state,
        )
    }
}

impl LevelHost for ReadLevel<'_, '_> {
    fn block_state(&self, [x, y, z]: [i32; 3]) -> Option<mod_api::BlockState> {
        self.game.world.block(x, y, z).try_into_api().ok()
    }
    fn state_block(&self, state: mod_api::BlockState) -> Option<&str> {
        self.game
            .pack
            .assets
            .registry
            .state(state.try_into_engine(&()).ok()?)
            .map(|s| &*s.name)
    }
    fn block_state_id(&self, name: &str) -> Option<mod_api::BlockState> {
        self.game.block_state_id(name)?.try_into_api().ok()
    }
    fn block_state_default(&self, block: mod_api::BlockKey) -> Option<mod_api::BlockState> {
        self.game.block_state_default(block)?.try_into_api().ok()
    }
    fn state_string(&self, state: mod_api::BlockState) -> Option<String> {
        self.game
            .block_state_string(state.try_into_engine(&()).ok()?)
    }
    fn min_y(&self) -> i32 {
        self.game.world.shape.min_y
    }
    fn height(&self) -> i32 {
        self.game.world.shape.height
    }
    fn block_in_tag(&self, block: mod_api::BlockKey, tag: &str) -> Option<bool> {
        super::native_world_read::block_in_tag(block, tag)
    }
    fn tags(&self) -> &dyn mod_api::Tags {
        self
    }
    fn fluid_state(&self, [x, y, z]: [i32; 3]) -> Option<mod_api::FluidState> {
        self.game
            .world
            .is_loaded_at(x, y, z)
            .then(|| self.game.world.block(x, y, z))
            .and_then(|state| self.game.state_fluid_state(state))
    }
    fn state_fluid_state(&self, state: mod_api::BlockState) -> Option<mod_api::FluidState> {
        self.game
            .state_fluid_state(state.try_into_engine(&()).ok()?)
    }
    fn fluid_in_tag(&self, fluid: &str, tag: &str) -> Option<bool> {
        Some(super::native_world_read::fluid_in_tag(
            self.game.items(),
            fluid,
            tag,
        ))
    }
    fn state_with_property(
        &self,
        state: mod_api::BlockState,
        key: &str,
        value: &str,
    ) -> Option<mod_api::BlockState> {
        super::native_hooks::with_property(
            self.game.pack.assets.registry,
            state.try_into_engine(&()).ok()?,
            key,
            value,
        )?
        .try_into_api()
        .ok()
    }
    fn brightness(&self, layer: mod_api::LightLayer, pos: [i32; 3]) -> i32 {
        super::native_level_refs::world_brightness(&self.game.world, layer, pos)
    }
    fn raw_brightness(&self, pos: [i32; 3], sky_darken: i32) -> i32 {
        self.brightness(mod_api::LightLayer::Block, pos)
            .max(self.brightness(mod_api::LightLayer::Sky, pos) - sky_darken)
    }
    fn is_loaded(&self, [x, y, z]: [i32; 3]) -> bool {
        self.game.world.is_loaded_at(x, y, z)
    }
    fn is_raining_at(&self, pos: [i32; 3]) -> bool {
        self.game
            .level_is_raining_at(self.level, pos)
            .unwrap_or(false)
    }
    fn enabled_features(&self) -> mod_api::FeatureFlags {
        mod_api::FeatureFlags::new(self.game.enabled_feature_ids().clone())
    }
}
