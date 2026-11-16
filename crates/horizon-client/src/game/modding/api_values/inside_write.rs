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

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_world::registry::BlockRegistry;

    #[test]
    fn inside_batch_stages_in_order_and_refuses_invalid_states() {
        let registry = BlockRegistry::from_facts_json(
            br#"{"defaults":{"minecraft:air":0,"minecraft:stone":1,"minecraft:dirt":2},"states":[["air",true,false,"","minecraft:air"],["s",false,false,"","minecraft:stone"],["d",false,false,"","minecraft:dirt"]]}"#
                .as_slice(),
        )
        .unwrap();
        super::super::resolve_block_definitions(&registry).unwrap();
        let stone = BlockStateId(1).try_into_api().unwrap();
        let dirt = BlockStateId(2).try_into_api().unwrap();
        let requested = vec![
            InsideWrite::SetBlock(stone),
            InsideWrite::DestroyBlock { drop: true },
            InsideWrite::SetBlock(dirt),
            InsideWrite::DestroyBlock { drop: false },
        ];
        let expected = vec![
            EngineWrite::SetBlock(BlockStateId(1)),
            EngineWrite::DestroyBlock { drop: true },
            EngineWrite::SetBlock(BlockStateId(2)),
            EngineWrite::DestroyBlock { drop: false },
        ];
        assert_eq!(requested.try_into_engine(&()).unwrap(), expected);
        assert_eq!(expected.try_into_api().unwrap(), requested);
        assert_eq!(
            EngineWrite::SetBlock(BlockStateId(u32::MAX)).try_into_api(),
            Err(BlockStateError::UnknownState)
        );
        for invalid in [
            mod_api::BlockStateRecord::new(stone.block(), u16::MAX).into(),
            mod_api::BlockStateRecord::new(stone.block(), u16::MAX - 1).into(),
        ] {
            // A valid prefix and suffix must not leak into an existing engine queue.
            let batch = vec![requested[0], InsideWrite::SetBlock(invalid), requested[3]];
            let mut queue = vec![EngineWrite::DestroyBlock { drop: true }];
            let staged = batch.try_into_engine(&());
            assert_eq!(staged, Err(BlockStateError::UnknownState));
            if let Ok(staged) = staged {
                queue.extend(staged);
            }
            assert_eq!(queue, vec![EngineWrite::DestroyBlock { drop: true }]);
        }
    }
}
