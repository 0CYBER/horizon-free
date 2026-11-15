//! Callbacks into the executable for hooks fired while a feature generates: on
//! the worldgen workers, and on the server thread when a sapling grows. No game
//! is at hand there, so a hook gets the server's [`WorldgenContext`] instead.
use crate::{
    native_load_context::WorldgenContext, registry::BlockStateId, worldgen::structure::Rotation,
};
use std::sync::{Arc, OnceLock, PoisonError, RwLock};

/// The random of the feature that asks a mod, as the mod draws from it: its
/// draws fall between the feature's own, in call order, so the same seed gives
/// the same world.
pub trait WorldgenRandom {
    /// `nextInt(bound)`; 0 for a bound below 1.
    fn next_int_bound(&mut self, bound: i32) -> i32;
    /// `nextLong`.
    fn next_long(&mut self) -> i64;
    /// `nextFloat`.
    fn next_float(&mut self) -> f32;
    /// `nextDouble`.
    fn next_double(&mut self) -> f64;
    /// `nextBoolean`.
    fn next_boolean(&mut self) -> bool;
}

/// What a mod's ground provider reads while the tree decorator asks it for
/// one ground position: the region being decorated, the decorator's own random
/// (vanilla's `TreeDecorator.Context.random()`, so draws keep vanilla's order)
/// and the decorator's own provider.
pub trait GroundAccess: WorldgenRandom {
    /// The block at `pos` in the region being decorated.
    fn block_state(&mut self, pos: [i32; 3]) -> BlockStateId;
    /// The decorator's own provider at `pos` (`BlockStateProvider.getOptionalState`),
    /// drawing from the same random: None where it places nothing.
    fn decorator_state(&mut self, pos: [i32; 3]) -> Option<BlockStateId>;
}

/// What the `on_tree_grow` parts read while a trunk placer asks them for the
/// block under its trunk: the region the tree grows in and the tree's own random
/// (`TrunkPlacer.placeBelowTrunkBlock`'s).
pub trait TreeGrowAccess: WorldgenRandom {
    /// The block at `pos` in the region the tree grows in; None where it cannot
    /// be read, an error the trunk placer reports once the parts answered.
    fn block_state(&mut self, pos: [i32; 3]) -> Option<BlockStateId>;
}

/// A ground provider a mod put in place of the tree decorator's own, as the
/// executable's adapter hands it over. `Send + Sync` because neighbouring
/// chunks decorate at once on worker threads.
pub trait ModGroundProvider: Send + Sync {
    /// The state to place at `pos`, or None to try the next position down the
    /// column (`AlterGroundEvent.StateProvider.getState`).
    fn state_at(&self, access: &mut dyn GroundAccess, pos: [i32; 3]) -> Option<BlockStateId>;
}

/// The tree whose ground `AlterGroundDecorator.place` alters: the decorator
/// context's lists (`TreeDecorator.Context`), each sorted by Y, and the
/// positions it places around (`TreeFeature.getLowestTrunkOrRootOfTree`).
pub struct AlterGroundTree<'a> {
    pub logs: &'a [[i32; 3]],
    pub leaves: &'a [[i32; 3]],
    pub roots: &'a [[i32; 3]],
    /// Never empty: the decorator fires nothing for a tree without them.
    pub positions: &'a [[i32; 3]],
}

/// Feature-generation hooks, installed before any world opens.
pub struct NativeWorldgenHooks {
    /// `alter_ground`: leaves `provider` None to keep the decorator's own.
    pub alter_ground: fn(&AlterGroundTree<'_>, &mut Option<Arc<dyn ModGroundProvider>>),
    /// `on_tree_grow`: the block the parts place under a growing trunk at `pos`,
    /// `Some(None)` for none, or None to leave it to the tree's own
    /// `below_trunk_provider`.
    #[allow(clippy::option_option)]
    pub on_tree_grow: fn(&mut dyn TreeGrowAccess, [i32; 3]) -> Option<Option<BlockStateId>>,
    /// `rotate`: the state a template places at `pos` for `state`, already mirrored, turned
    /// by `rotation`, starting from `table`, the game's own turned state.
    pub rotate: fn(BlockStateId, [i32; 3], Rotation, BlockStateId) -> BlockStateId,
    /// Runs one feature, whose Part lookups read the tables the server published
    /// when it started: a feature that started before a `/reload` finishes with
    /// the tables it started with.
    pub feature: fn(&mut dyn FnMut()),
}

static HOOKS: OnceLock<NativeWorldgenHooks> = OnceLock::new();
static CONTEXT: RwLock<Option<Arc<WorldgenContext>>> = RwLock::new(None);

/// Install the process-wide callbacks once; a second installation keeps the first.
pub fn install(hooks: NativeWorldgenHooks) -> Result<(), NativeWorldgenHooks> {
    HOOKS.set(hooks)
}

/// Publish the server's context for the features it generates from now on,
/// replacing the one before. Called where the server starts or reloads its
/// resources.
pub fn publish_context(context: Arc<WorldgenContext>) {
    *CONTEXT.write().unwrap_or_else(PoisonError::into_inner) = Some(context);
}

/// The context the server published last; None before a server published one.
pub fn context() -> Option<Arc<WorldgenContext>> {
    CONTEXT
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

/// The provider mods put in place of the decorator's own for `tree`, or None
/// to keep the decorator's own.
pub(crate) fn alter_ground(tree: &AlterGroundTree<'_>) -> Option<Arc<dyn ModGroundProvider>> {
    let hooks = HOOKS.get()?;
    let mut provider = None;
    (hooks.alter_ground)(tree, &mut provider);
    provider
}

/// The block the parts of the block at `pos` place under a growing trunk:
/// `Some(None)` for none, or None to leave it to the tree's own provider.
#[allow(clippy::option_option)]
pub(crate) fn on_tree_grow(
    access: &mut dyn TreeGrowAccess,
    pos: [i32; 3],
) -> Option<Option<BlockStateId>> {
    (HOOKS.get()?.on_tree_grow)(access, pos)
}

/// The state the parts of `state`'s block place at `pos` for `state` turned by `rotation`,
/// starting from the game's own `table`; `table` before the hooks are installed.
pub(crate) fn rotate(
    state: BlockStateId,
    pos: [i32; 3],
    rotation: Rotation,
    table: BlockStateId,
) -> BlockStateId {
    match HOOKS.get() {
        Some(hooks) => (hooks.rotate)(state, pos, rotation, table),
        None => table,
    }
}

/// Run one feature with the Part tables published when it started.
pub(crate) fn feature<T>(place: impl FnOnce() -> T) -> T {
    let Some(hooks) = HOOKS.get() else {
        return place();
    };
    let mut place = Some(place);
    let mut placed = None;
    (hooks.feature)(&mut || placed = place.take().map(|place| place()));
    match (placed, place) {
        (Some(placed), _) => placed,
        // The callback did not run it: run it here, with the tables at hand.
        (None, Some(place)) => place(),
        (None, None) => unreachable!("a feature that ran has a result"),
    }
}
