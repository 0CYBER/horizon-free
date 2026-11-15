//! Resolved state-only reactions; properties remain intact for subsequent reloads.
use super::{Behaviours, PushReaction};
use crate::registry::BlockStateId;
use std::sync::atomic::{AtomicU8, Ordering};

// Resources are shared by the server and replay threads. Reload updates only this
// scalar; each answer must be the same pure function of state on both sides.
#[derive(Debug)]
pub(crate) struct CachedReaction(AtomicU8);
impl Clone for CachedReaction {
    fn clone(&self) -> Self {
        Self(AtomicU8::new(self.0.load(Ordering::Relaxed)))
    }
}
impl CachedReaction {
    pub(crate) fn new(value: PushReaction) -> Self {
        Self(AtomicU8::new(value as u8))
    }
    fn get(&self) -> PushReaction {
        match self.0.load(Ordering::Relaxed) {
            0 => PushReaction::PushPull,
            1 => PushReaction::Push,
            2 => PushReaction::Popped,
            3 => PushReaction::Immoveable,
            4 => PushReaction::IgnoreEntity,
            _ => unreachable!("only closed engine reactions are stored"),
        }
    }
}
impl Behaviours {
    /// Refresh after part installation on load and reload. None restores properties.
    pub fn resolve_piston_reactions(
        &self,
        mut answer: impl FnMut(BlockStateId) -> Option<PushReaction>,
    ) {
        for (index, facts) in self.facts.iter().enumerate() {
            let value = answer(BlockStateId(index as u32)).unwrap_or(facts.push);
            facts.resolved_push.0.store(value as u8, Ordering::Relaxed);
        }
    }
    /// One cached read for server simulation and client replay.
    pub fn piston_reaction(&self, state: BlockStateId) -> PushReaction {
        self.facts(state).resolved_push.get()
    }
}
