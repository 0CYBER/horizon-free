//! One vine attachment in native direction priority order.
use super::{
    DecodeError,
    block_feature::{BlockFeatureError, FeatureBlockBindings},
    feature::FeatureWorld,
    placement::PlacementContext,
};
use crate::registry::BlockStateId;
use horizon_core::geometry::Direction;
const DIRECTIONS: [Direction; 5] = [
    Direction::Up,
    Direction::North,
    Direction::South,
    Direction::West,
    Direction::East,
];
#[derive(Clone, Debug)]
pub struct VinesFeature {
    air: [BlockStateId; 3],
    states: [BlockStateId; 5],
}
impl VinesFeature {
    pub fn new(bindings: &FeatureBlockBindings<'_>) -> Result<Self, DecodeError> {
        let registry = bindings.registry();
        let default = registry
            .default_state("minecraft:vine")
            .ok_or(DecodeError::Field("vine block missing"))?;
        let mut states = [default; 5];
        for (i, name) in ["up", "north", "south", "west", "east"]
            .into_iter()
            .enumerate()
        {
            let mut state = registry.state(default).unwrap().clone();
            state.properties.insert(name.into(), "true".into());
            states[i] = registry
                .id(&state)
                .ok_or(DecodeError::Field("vine face missing"))?;
        }
        Ok(Self {
            air: bindings.air,
            states,
        })
    }
    pub fn place<W: FeatureWorld + PlacementContext>(
        &self,
        world: &mut W,
        p: [i32; 3],
    ) -> Result<bool, BlockFeatureError<W::Error>> {
        if !self
            .air
            .contains(&world.block(p).map_err(BlockFeatureError::World)?)
        {
            return Ok(false);
        }
        for (direction, state) in DIRECTIONS.into_iter().zip(self.states) {
            if world
                .attachment_at(p, direction)
                .map_err(BlockFeatureError::Query)?
            {
                world
                    .set_block(p, state, 2)
                    .map_err(BlockFeatureError::World)?;
                return Ok(true);
            }
        }
        Ok(false)
    }
}
