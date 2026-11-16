//! The ground a tree decorator places (vanilla's `AlterGroundDecorator`, the
//! podzol under a big spruce), and the provider a mod puts in its place in
//! `alter_ground` (NeoForge's `AlterGroundEvent`); the block a trunk places
//! under itself, which the soil's parts answer in `on_tree_grow`.
//!
//! The decorator asks its provider once per ground position it tries, on the
//! worldgen worker threads (and on the server thread when a sapling grows), so a
//! provider keeps no mutable state and gets no `Host`: it reads the region the
//! tree stands in and draws from the decorator's own random through
//! [`GroundLevel`], so with the decorator's own provider kept, the blocks and the
//! draws are vanilla's.
use crate::{BlockState, BlockStateId, StructureRandom};
use std::sync::Arc;

/// A mod's ground provider: NeoForge's `AlterGroundEvent.StateProvider`.
/// Implemented once, kept by the game, deterministic and stateless.
pub trait GroundStateProvider: Send + Sync + 'static {
    /// `StateProvider.getState`: the state to place at `pos`, or None to try
    /// the next position down the column, as the decorator's own provider
    /// answers where it finds no ground it replaces. Draw only from
    /// `level.random()`, so the same seed gives the same ground.
    fn state_at(&self, level: &mut dyn GroundLevel, pos: [i32; 3]) -> Option<BlockStateId>;
}

/// What a ground provider reads while the decorator asks it: vanilla's
/// `WorldGenLevel` as far as ground needs it, and the decorator's random.
pub trait GroundLevel {
    /// `BlockGetter.getBlockState`: the block at `pos` in the region the tree
    /// decorates.
    fn block_state(&mut self, pos: [i32; 3]) -> BlockStateId;
    /// The decorator's own provider at `pos`
    /// (`BlockStateProvider.getOptionalState`), drawing from the same random:
    /// what [`GroundProvider::Decorator`] answers.
    fn decorator_state(&mut self, pos: [i32; 3]) -> Option<BlockStateId>;
    /// The decorator's random (`TreeDecorator.Context.random()`): a provider's
    /// draws come between the decorator's own, in the order it asks.
    fn random(&mut self) -> &mut dyn StructureRandom;
}

/// The provider a tree decorator places its ground with:
/// `AlterGroundEvent.getStateProvider`.
#[derive(Clone)]
// exhaustive: the decorator's own or a mod's; a third kind is a contract change
#[derive(horizon_mod_api_macros::Wire)]
pub enum GroundProvider {
    /// The decorator's own (`AlterGroundDecorator.provider`).
    Decorator,
    /// A mod's, in place of the one before. To change the ground only where
    /// it likes, a mod wraps the provider it found and asks it for the rest.
    Mod(Arc<dyn GroundStateProvider>),
}

/// The same provider: both the decorator's own, or the same mod provider object.
impl PartialEq for GroundProvider {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Decorator, Self::Decorator) => true,
            (Self::Mod(a), Self::Mod(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

impl GroundProvider {
    /// This provider's state at `pos`, or None to try the next position down.
    pub fn state_at(&self, level: &mut dyn GroundLevel, pos: [i32; 3]) -> Option<BlockStateId> {
        match self {
            Self::Decorator => level.decorator_state(pos),
            Self::Mod(provider) => provider.state_at(level, pos),
        }
    }
}

/// `AlterGroundDecorator.place` is about to alter the ground around
/// `positions`: the provider it places with is `provider`.
#[inline(never)]
pub fn native_alter_ground(
    context: &crate::OwningWorldgen<'_>,
    logs: &[[i32; 3]],
    leaves: &[[i32; 3]],
    roots: &[[i32; 3]],
    positions: &[[i32; 3]],
    provider: &mut GroundProvider,
) {
    crate::dispatch_alter_ground(context, logs, leaves, roots, positions, provider);
}

/// `TrunkPlacer.placeBelowTrunkBlock` is about to place the block under a
/// trunk at `pos`, where `state` stands: the parts of `state`'s block answer
/// `soil`, drawing from the tree's `random`.
#[inline(never)]
#[allow(clippy::option_option)]
pub fn native_on_tree_grow(
    context: &crate::OwningWorldgen<'_>,
    state: BlockState,
    pos: [i32; 3],
    random: &mut dyn StructureRandom,
    soil: &mut Option<Option<BlockState>>,
) {
    crate::dispatch_on_tree_grow(context, state, pos, random, soil);
}
