//! Checked inside writes. Vector conversion stages the entire batch before dispatch commits it.
use super::{TryIntoApi, TryIntoEngine};
use horizon_world::{level::InsideWrite as EngineWrite, registry::BlockStateId};
use mod_api::{BlockState, BlockStateError, InsideWrite};

impl TryIntoApi for EngineWrite
where
    BlockStateId: TryIntoApi<Api = BlockState, Error = BlockStateError>,
{
    type Api = InsideWrite;
    type Error = BlockStateError;

    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        match self {
            EngineWrite::SetBlock(state) => Ok(InsideWrite::SetBlock(state.try_into_api()?)),
            EngineWrite::DestroyBlock { drop } => Ok(InsideWrite::DestroyBlock { drop: *drop }),
            _ => Err(BlockStateError::Unavailable),
        }
    }
}

impl<C: ?Sized> TryIntoEngine<C> for InsideWrite
where
    BlockState: TryIntoEngine<C, Engine = BlockStateId, Error = BlockStateError>,
{
    type Engine = EngineWrite;
    type Error = BlockStateError;

    fn try_into_engine(&self, context: &C) -> Result<Self::Engine, Self::Error> {
        match self {
            Self::SetBlock(state) => Ok(EngineWrite::SetBlock(state.try_into_engine(context)?)),
            Self::DestroyBlock { drop } => Ok(EngineWrite::DestroyBlock { drop: *drop }),
        }
    }
}
