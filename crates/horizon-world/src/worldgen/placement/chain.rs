//! Ordered modifier-chain execution with reusable native-style pending buffers.
use super::{PlacementContext, PlacementError, PlacementModifier, PlacementQueryError};
use crate::worldgen::anchor::WorldGenerationContext;
use crate::{heightmap::HeightmapType, worldgen::feature_sort::FeatureIdentity};
use horizon_core::random::{InvalidBound, RandomSource};
use std::cell::Cell;

struct Queries<'a, C> {
    world: &'a mut C,
    top: Option<FeatureIdentity>,
}
impl<C: PlacementContext> PlacementContext for Queries<'_, C> {
    fn block_survives(
        &mut self,
        state: crate::registry::BlockStateId,
        position: [i32; 3],
    ) -> Result<bool, PlacementQueryError> {
        self.world.block_survives(state, position)
    }
    fn face_sturdy(
        &mut self,
        position: [i32; 3],
        direction: horizon_core::geometry::Direction,
    ) -> Result<bool, PlacementQueryError> {
        self.world.face_sturdy(position, direction)
    }
    fn unobstructed_block(&mut self, position: [i32; 3]) -> Result<bool, PlacementQueryError> {
        self.world.unobstructed_block(position)
    }
    fn biome_identity(
        &mut self,
        position: [i32; 3],
    ) -> Result<crate::worldgen::biome::BiomeId, PlacementQueryError> {
        self.world.biome_identity(position)
    }
    fn top_feature(&self) -> Option<FeatureIdentity> {
        self.top
    }
    fn biome_allows_feature(
        &mut self,
        p: [i32; 3],
        feature: FeatureIdentity,
    ) -> Result<bool, PlacementQueryError> {
        self.world.biome_allows_feature(p, feature)
    }
    fn generation(&self) -> WorldGenerationContext {
        self.world.generation()
    }
    fn predicate_generation(&mut self) -> Result<WorldGenerationContext, PlacementQueryError> {
        self.world.predicate_generation()
    }
    fn block_state(
        &mut self,
        position: [i32; 3],
    ) -> Result<crate::registry::BlockStateId, PlacementQueryError> {
        self.world.block_state(position)
    }
    fn block_matches_tag(
        &mut self,
        position: [i32; 3],
        tag: &str,
    ) -> Result<bool, PlacementQueryError> {
        self.world.block_matches_tag(position, tag)
    }
    fn min_y(&mut self) -> Result<i32, PlacementQueryError> {
        self.world.min_y()
    }
    fn max_y(&mut self) -> Result<i32, PlacementQueryError> {
        self.world.max_y()
    }
    fn height(&mut self, kind: HeightmapType, x: i32, z: i32) -> Result<i32, PlacementQueryError> {
        self.world.height(kind, x, z)
    }
    fn layer_block(&mut self, p: [i32; 3]) -> Result<super::LayerBlock, PlacementQueryError> {
        self.world.layer_block(p)
    }
    fn inside_build_height(&mut self, p: [i32; 3]) -> Result<bool, PlacementQueryError> {
        self.world.inside_build_height(p)
    }
    fn outside_build_height(&mut self, y: i32) -> Result<bool, PlacementQueryError> {
        self.world.outside_build_height(y)
    }
}

#[derive(Debug)]
pub enum FeaturePlacementError<E> {
    Random(InvalidBound),
    World(PlacementQueryError),
    Feature(E),
    WorkLimit,
    InvalidPendingIndex,
}
impl<E: std::fmt::Display> std::fmt::Display for FeaturePlacementError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Random(e) => e.fmt(f),
            Self::World(e) => e.fmt(f),
            Self::Feature(e) => e.fmt(f),
            Self::WorkLimit => f.write_str("feature placement work limit exceeded"),
            Self::InvalidPendingIndex => {
                f.write_str("pending modifier index is outside the current chain")
            }
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for FeaturePlacementError<E> {}

/// Processes one modifier completely before running its descendants. Siblings
/// retain emitted order. The reusable buffers also retain partial state after
/// errors, like vanilla; a fresh placer starts without that state.
#[derive(Default, Debug)]
pub struct FeaturePlacer {
    pending: Vec<([i32; 3], usize)>,
    modified: Vec<[i32; 3]>,
}
impl FeaturePlacer {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn place<R: RandomSource, E>(
        &mut self,
        modifiers: &[PlacementModifier],
        mut context: WorldGenerationContext,
        random: &mut R,
        origin: [i32; 3],
        feature: impl FnMut(&mut R, [i32; 3]) -> Result<bool, E>,
    ) -> Result<bool, FeaturePlacementError<E>> {
        self.place_in(modifiers, &mut context, random, origin, feature)
    }
    pub fn place_in<R: RandomSource, E>(
        &mut self,
        modifiers: &[PlacementModifier],
        context: &mut impl PlacementContext,
        random: &mut R,
        origin: [i32; 3],
        mut feature: impl FnMut(&mut R, [i32; 3]) -> Result<bool, E>,
    ) -> Result<bool, FeaturePlacementError<E>> {
        self.place_with_context(modifiers, context, random, origin, |_, random, position| {
            feature(random, position)
        })
    }
    /// Shares the mutable world and random stream between modifier queries and
    /// feature writes. Later siblings observe earlier feature mutations.
    pub fn place_with_context<C: PlacementContext, R: RandomSource, E>(
        &mut self,
        modifiers: &[PlacementModifier],
        context: &mut C,
        random: &mut R,
        origin: [i32; 3],
        feature: impl FnMut(&mut C, &mut R, [i32; 3]) -> Result<bool, E>,
    ) -> Result<bool, FeaturePlacementError<E>> {
        self.place_with_budget(
            modifiers,
            context,
            random,
            origin,
            &Cell::new(1 << 20),
            feature,
        )
    }
    /// Set the root placed feature only for this invocation's modifier queries.
    /// Nested feature callbacks receive the original world, so their own plain
    /// placers do not inherit this root's biome-check context.
    pub fn place_with_biome_check<C: PlacementContext, R: RandomSource, E>(
        &mut self,
        modifiers: &[PlacementModifier],
        context: &mut C,
        random: &mut R,
        origin: [i32; 3],
        top_feature: FeatureIdentity,
        feature: impl FnMut(&mut C, &mut R, [i32; 3]) -> Result<bool, E>,
    ) -> Result<bool, FeaturePlacementError<E>> {
        self.place_control(
            modifiers,
            context,
            random,
            origin,
            (&Cell::new(1 << 20), Some(top_feature)),
            feature,
        )
    }
    pub(crate) fn place_with_budget<C: PlacementContext, R: RandomSource, E>(
        &mut self,
        modifiers: &[PlacementModifier],
        context: &mut C,
        random: &mut R,
        origin: [i32; 3],
        remaining: &Cell<usize>,
        feature: impl FnMut(&mut C, &mut R, [i32; 3]) -> Result<bool, E>,
    ) -> Result<bool, FeaturePlacementError<E>> {
        self.place_control(
            modifiers,
            context,
            random,
            origin,
            (remaining, None),
            feature,
        )
    }
    fn place_control<C: PlacementContext, R: RandomSource, E>(
        &mut self,
        modifiers: &[PlacementModifier],
        context: &mut C,
        random: &mut R,
        origin: [i32; 3],
        control: (&Cell<usize>, Option<FeatureIdentity>),
        mut feature: impl FnMut(&mut C, &mut R, [i32; 3]) -> Result<bool, E>,
    ) -> Result<bool, FeaturePlacementError<E>> {
        let (remaining, top) = control;
        const LIMIT: usize = 1 << 20;
        if modifiers.len() > 1024 || self.pending.len() >= LIMIT {
            return Err(FeaturePlacementError::WorkLimit);
        }
        if modifiers.is_empty() {
            return feature(context, random, origin).map_err(FeaturePlacementError::Feature);
        }
        self.pending.push((origin, 0));
        let mut any = false;
        while let Some((position, index)) = self.pending.pop() {
            if remaining.get() == 0 {
                return Err(FeaturePlacementError::WorkLimit);
            }
            remaining.set(remaining.get() - 1);
            let modifier = modifiers
                .get(index)
                .ok_or(FeaturePlacementError::InvalidPendingIndex)?;
            modifier
                .modify_in(
                    &mut Queries {
                        world: context,
                        top,
                    },
                    random,
                    position,
                    |_, p| {
                        if self.modified.len() >= LIMIT {
                            return Err(());
                        }
                        self.modified.push(p);
                        Ok(())
                    },
                )
                .map_err(|e| match e {
                    PlacementError::Random(e) => FeaturePlacementError::Random(e),
                    PlacementError::World(e) => FeaturePlacementError::World(e),
                    PlacementError::Output(()) | PlacementError::WorkLimit => {
                        FeaturePlacementError::WorkLimit
                    }
                })?;
            let next = index + 1;
            if next < modifiers.len() {
                if self.pending.len() + self.modified.len() > LIMIT {
                    return Err(FeaturePlacementError::WorkLimit);
                }
                self.pending
                    .extend(self.modified.iter().rev().map(|p| (*p, next)));
            } else {
                for p in &self.modified {
                    if remaining.get() == 0 {
                        return Err(FeaturePlacementError::WorkLimit);
                    }
                    remaining.set(remaining.get() - 1);
                    any |= feature(context, random, *p).map_err(FeaturePlacementError::Feature)?;
                }
            }
            self.modified.clear();
        }
        Ok(any)
    }
}
