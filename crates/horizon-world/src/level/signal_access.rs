//! The shared inputs of [`super::Signals`], without a storage write or a random source.
use super::LevelAccess;
use crate::registry::BlockStateId;
use horizon_core::{geometry::Direction, position::BlockPos};

/// The level reads and block callbacks used by the signal evaluator.
/// A shared owner implements only these reads; mutable simulations delegate their
/// existing [`LevelAccess`] through the blanket implementation below.
pub trait SignalAccess {
    /// The state at `pos`, including air outside loaded storage.
    fn block_state(&self, pos: BlockPos) -> BlockStateId;
    /// The current shulker lid state used by wire support checks.
    fn shulker_is_closed(&self, pos: BlockPos) -> Option<bool>;
    /// The block's signal-source answer after its Part.
    fn adjust_signal_source(&self, state: BlockStateId, vanilla: bool) -> bool;
    /// Whether weak-power queries have a subscribed Part.
    fn has_weak_power_hook(&self) -> bool;
    /// The conductor answer after the block's weak-power Part.
    fn adjust_should_check_weak_power(
        &self,
        pos: BlockPos,
        state: BlockStateId,
        direction: Direction,
        vanilla: bool,
    ) -> bool;
    /// Whether wire-connection queries have a subscribed Part.
    fn has_redstone_wire_connection_hook(&self) -> bool;
    /// The wire-connection answer after the block's Part.
    fn adjust_should_redstone_wire_connect_to(
        &self,
        pos: BlockPos,
        state: BlockStateId,
        direction: Option<Direction>,
        vanilla: bool,
    ) -> bool;
    /// The weak or strong output after the block's Part.
    fn adjust_signal(
        &self,
        pos: BlockPos,
        state: BlockStateId,
        direction: Direction,
        direct: bool,
        vanilla: i32,
    ) -> i32;
    /// The output of a source whose value lives in its block entity.
    fn block_entity_signal(&self, pos: BlockPos, state: BlockStateId, direction: Direction) -> i32;
}

impl<L: LevelAccess + ?Sized> SignalAccess for L {
    fn block_state(&self, pos: BlockPos) -> BlockStateId {
        LevelAccess::block_state(self, pos)
    }
    fn shulker_is_closed(&self, pos: BlockPos) -> Option<bool> {
        LevelAccess::shulker_is_closed(self, pos)
    }
    fn adjust_signal_source(&self, state: BlockStateId, vanilla: bool) -> bool {
        LevelAccess::adjust_signal_source(self, state, vanilla)
    }
    fn has_weak_power_hook(&self) -> bool {
        LevelAccess::has_weak_power_hook(self)
    }
    fn adjust_should_check_weak_power(
        &self,
        pos: BlockPos,
        state: BlockStateId,
        direction: Direction,
        vanilla: bool,
    ) -> bool {
        LevelAccess::adjust_should_check_weak_power(self, pos, state, direction, vanilla)
    }
    fn has_redstone_wire_connection_hook(&self) -> bool {
        LevelAccess::has_redstone_wire_connection_hook(self)
    }
    fn adjust_should_redstone_wire_connect_to(
        &self,
        pos: BlockPos,
        state: BlockStateId,
        direction: Option<Direction>,
        vanilla: bool,
    ) -> bool {
        LevelAccess::adjust_should_redstone_wire_connect_to(self, pos, state, direction, vanilla)
    }
    fn adjust_signal(
        &self,
        pos: BlockPos,
        state: BlockStateId,
        direction: Direction,
        direct: bool,
        vanilla: i32,
    ) -> i32 {
        LevelAccess::adjust_signal(self, pos, state, direction, direct, vanilla)
    }
    fn block_entity_signal(&self, pos: BlockPos, state: BlockStateId, direction: Direction) -> i32 {
        LevelAccess::block_entity_signal(self, pos, state, direction)
    }
}
