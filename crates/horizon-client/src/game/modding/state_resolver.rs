//! The shared per-state resolver: builds a table from a state-only `BlockPart` method, one entry
//! per block state or per (state, fluid) pair, so the code that reads it (path finding, the
//! meshing workers) dispatches to no mod. A table is built when parts install (world load,
//! `/reload`) or when its owner refreshes it, never per tick or per frame, and only where a part
//! subscribes: with none, there is no table and every reader keeps the block's own answer.

use super::{BlockStateId, Game};
use mod_api::{__private::has_scoped_hook, HookId};

impl Game<'_> {
    /// `ask` for each of the first `states` block states, in id order, when a part subscribes
    /// to `hook`; `None` when none does, and nothing is asked.
    pub(in crate::game) fn resolve_per_state<T>(
        hook: HookId,
        states: usize,
        mut ask: impl FnMut(BlockStateId) -> T,
    ) -> Option<Box<[T]>> {
        has_scoped_hook(hook).then(|| {
            (0..states)
                .map(|state| ask(BlockStateId(state as u32)))
                .collect()
        })
    }
    /// `ask` for each of the first `states` block states and each fluid index below `fluids`,
    /// when a part subscribes to `hook`: the answer for (`state`, `fluid`) is at
    /// `state * fluids + fluid`. `None` when no part subscribes, and nothing is asked.
    pub(in crate::game) fn resolve_per_state_fluid<T>(
        hook: HookId,
        states: usize,
        fluids: usize,
        mut ask: impl FnMut(BlockStateId, usize) -> T,
    ) -> Option<Vec<T>> {
        has_scoped_hook(hook).then(|| {
            (0..states)
                .flat_map(|state| (0..fluids).map(move |fluid| (state, fluid)))
                .map(|(state, fluid)| ask(BlockStateId(state as u32), fluid))
                .collect()
        })
    }
}
