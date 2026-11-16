//! Immutable block metadata and registry facts (`host.block_states()`).
//! Queries consume owned API block states, never world numeric IDs. Both sides read the
//! game's published tables; definitions are copied out and property values are borrowed.
use crate::{
    Backend, BackendRef, BlockKey, BlockMetadata, BlockSoundType, BlockState, BlockStateError,
    BlockStateRange, BlockStateRecord, Host,
};

/// One property and its legal values in a state definition.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct StateProperty {
    /// Property name.
    pub name: String,
    /// Legal serialized values.
    pub values: Vec<String>,
}

impl StateProperty {
    /// Construct a definition entry from a property name and legal values.
    pub fn new(name: String, values: Vec<String>) -> Self {
        Self { name, values }
    }
}

/// A property constraint used when matching a state.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct StateConstraint {
    /// Property name.
    pub name: String,
    /// Required serialized value.
    pub value: String,
}

impl StateConstraint {
    /// Construct a property constraint.
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
        }
    }
}

/// An invalid constraint, identifying its index in the supplied list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum StateMatchError {
    /// The definition has no such property.
    UnknownProperty(u32),
    /// The property does not admit this value.
    InvalidValue(u32),
}

// no wire: borrowed host or registrar backend; access stays scoped to the owning call.
/// The block-state facts service: `host.block_states()`.
// exhaustive: its field is private; only `host.block_states()` builds one
pub struct BlockStates<'a> {
    backend: Option<&'a dyn Backend>,
}

/// Reaches [`BlockStates`] from any host.
pub trait HostBlockStates {
    /// Facts about block states, as the game's tables hold them.
    fn block_states(&self) -> BlockStates<'_>;
}

impl<H: Host + ?Sized> HostBlockStates for H {
    fn block_states(&self) -> BlockStates<'_> {
        BlockStates {
            backend: self.backend().map(BackendRef::open),
        }
    }
}

impl BlockMetadata for BlockStates<'_> {
    fn name(&self, block: BlockKey) -> Result<&str, BlockStateError> {
        self.backend
            .ok_or(BlockStateError::Unavailable)?
            .block_metadata_name(block)
    }
    fn property(
        &self,
        state: BlockStateRecord,
        name: &str,
    ) -> Result<Option<&str>, BlockStateError> {
        self.backend
            .ok_or(BlockStateError::Unavailable)?
            .block_metadata_property(state, name)
    }
    fn with(
        &self,
        state: BlockStateRecord,
        name: &str,
        value: &str,
    ) -> Result<BlockStateRecord, BlockStateError> {
        self.backend
            .ok_or(BlockStateError::Unavailable)?
            .block_metadata_with(state, name, value)
    }
    fn of(
        &self,
        block: BlockKey,
        properties: &[StateConstraint],
    ) -> Result<BlockStateRecord, BlockStateError> {
        self.backend
            .ok_or(BlockStateError::Unavailable)?
            .block_metadata_of(block, properties)
    }
}

impl BlockStates<'_> {
    /// `BlockState.canBeReplaced()`: whether placing a block where `state` stands replaces it
    /// (air, grass, water, snow layers). None for an unknown state, or a host without a game.
    pub fn replaceable(&self, state: BlockState) -> Option<bool> {
        self.backend?.block_state_replaceable(state)
    }
    /// How hard `state` is to break (`BlockBehaviour.Properties.destroyTime`, as
    /// `BlockState.getDestroySpeed` reads it): stone 1.5, obsidian 50, -1 for a block that
    /// can't be broken. None as for `replaceable`.
    pub fn destroy_speed(&self, state: BlockState) -> Option<f32> {
        self.backend?.block_state_destroy_speed(state)
    }
    /// The explosion resistance `state`'s block declares (`Block.getExplosionResistance`): stone
    /// 6, obsidian 1200, bedrock 3,600,000. What a blast meets at a position, parts and fluids
    /// included, is [`LevelRef::explosion_resistance`](crate::LevelRef::explosion_resistance).
    /// None as for `replaceable`.
    pub fn explosion_resistance(&self, state: BlockState) -> Option<f32> {
        self.backend?.block_state_explosion_resistance(state)
    }
    /// `state`'s sounds (`BlockState.getSoundType()`). None as for `replaceable`.
    pub fn sound(&self, state: BlockState) -> Option<BlockSoundType> {
        self.backend?.block_state_sound(state)
    }
    /// `BlockState.hasBlockEntity()`. None as for `replaceable`.
    pub fn has_block_entity(&self, state: BlockState) -> Option<bool> {
        self.backend?.block_state_has_block_entity(state)
    }
    /// `BlockState.getMapColor`: the state's `MapColor.id`, 0 for none (`MapColor.NONE`).
    /// None as for `replaceable`.
    pub fn map_color(&self, state: BlockState) -> Option<u8> {
        self.backend?.block_state_map_color(state)
    }
    /// The block's property names and legal serialized values. None without resolved tables.
    pub fn definition(&self, block: crate::BlockKey) -> Option<Vec<StateProperty>> {
        self.backend?.block_state_definition(block)
    }

    /// Match a state against a block and partial property constraints. Unknown properties and
    /// values are errors even when the state belongs to a different block. None means the
    /// owning thread has no resolved tables or the state is unknown.
    pub fn matches(
        &self,
        state: BlockState,
        block: crate::BlockKey,
        constraints: &[StateConstraint],
    ) -> Option<Result<bool, StateMatchError>> {
        self.backend?.block_state_matches(state, block, constraints)
    }

    /// The states of `block` (`minecraft:oak_stairs`), `StateDefinition.getPossibleStates`.
    /// Local ordinals retain their meaning if world numeric IDs reorder on reload.
    /// None before metadata is published, for an unknown block or for a host without a game.
    pub fn states_of(&self, block: crate::BlockKey) -> Option<BlockStateRange> {
        self.backend?.block_states_of(block)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A mod's test double: a host with no game behind it.
    struct Double;
    impl Host for Double {}
    mod_api::impl_host_areas!(Double);

    #[test]
    fn a_host_without_a_game_answers_none() {
        let facts = Double.block_states();
        let state: BlockState = BlockStateRecord::new(crate::BlockKey(0), 0).into();
        assert_eq!(facts.replaceable(state), None);
        assert_eq!(facts.destroy_speed(state), None);
        assert_eq!(facts.sound(state), None);
        assert_eq!(facts.has_block_entity(state), None);
        assert_eq!(facts.map_color(state), None);
        assert_eq!(facts.states_of(crate::BlockKey(0)), None);
    }
}
