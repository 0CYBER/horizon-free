//! Native simple-block feature selection, survival and ordered placement.
use super::feature::FeatureWorld;
use super::{
    DecodeError, Object, optional,
    placement::{PlacementContext, PlacementQueryError},
    required,
    state_provider::{
        BlockStateProvider, StateProviderBindings, StateProviderCatalog, StateProviderError,
    },
};
use crate::block_placement::{BlockPlacement, BlockPlacementBindings, BlockPlacementError};
use horizon_core::random::RandomSource;
use std::sync::Arc;

/// Immutable state providers and placement transitions share one registry.
#[derive(Clone, Debug)]
pub struct SimpleBlockFeature {
    provider: BlockStateProvider,
    placement: Arc<BlockPlacement>,
    schedule_tick: bool,
}
#[derive(Debug)]
pub enum SimpleBlockError<E> {
    Provider(StateProviderError<E>),
    Placement(BlockPlacementError<E>),
    Query(PlacementQueryError),
}
impl<E: std::fmt::Display> std::fmt::Display for SimpleBlockError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Provider(e) => e.fmt(f),
            Self::Placement(e) => e.fmt(f),
            Self::Query(e) => e.fmt(f),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for SimpleBlockError<E> {}
impl SimpleBlockFeature {
    /// Decode the feature's map codec, including optional named providers.
    pub fn from_json(
        json: &str,
        providers: &StateProviderBindings<'_>,
        catalog: &StateProviderCatalog,
        placement: &BlockPlacementBindings<'_>,
    ) -> Result<Self, DecodeError> {
        Self::decode(json, providers, catalog, placement, 0, &mut 65536)
    }
    pub(crate) fn decode(
        json: &str,
        providers: &StateProviderBindings<'_>,
        catalog: &StateProviderCatalog,
        placement: &BlockPlacementBindings<'_>,
        depth: usize,
        remaining: &mut usize,
    ) -> Result<Self, DecodeError> {
        if !std::ptr::eq(providers.registry(), placement.registry()) {
            return Err(DecodeError::Field("simple block registry identity"));
        }
        let object: Object = serde_json::from_str(json)?;
        let provider = catalog.provider_with_budget(
            required(&object, "to_place")?.get(),
            providers,
            depth + 1,
            remaining,
        )?;
        let schedule_tick = optional(&object, "schedule_tick")
            .map(|v| serde_json::from_str::<bool>(v.get()))
            .transpose()?
            .unwrap_or(false);
        Ok(Self {
            provider,
            placement: placement.operations(),
            schedule_tick,
        })
    }
    pub fn place<W: FeatureWorld + PlacementContext>(
        &self,
        world: &mut W,
        random: &mut impl RandomSource,
        position: [i32; 3],
    ) -> Result<bool, SimpleBlockError<W::Error>> {
        let Some(state) = self
            .provider
            .get_optional_state(world, random, position)
            .map_err(SimpleBlockError::Provider)?
        else {
            return Ok(false);
        };
        if !world
            .block_survives(state, position)
            .map_err(SimpleBlockError::Query)?
        {
            return Ok(false);
        }
        if !self
            .placement
            .place(world, state, position, 2)
            .map_err(SimpleBlockError::Placement)?
        {
            return Ok(false);
        }
        if self.schedule_tick {
            self.placement
                .schedule_tick(world, position, 1)
                .map_err(SimpleBlockError::Placement)?;
        }
        Ok(true)
    }
}
