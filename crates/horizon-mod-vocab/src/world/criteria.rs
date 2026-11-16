//! A player's mod-owned advancement triggers and their scoped predicate context.

use crate::{ItemStack, NoBackend};

/// One open criterion being tested during [`PlayerRef::trigger_criterion`](crate::PlayerRef::trigger_criterion).
///
/// The game lends this resource only for [`CriterionTest::test`]. Its private world context is
/// the target player's own level; a mod cannot retain the resource after the test returns.
/// The same native implementation reads conditions and evaluates item predicates for both
/// build targets.
pub trait CriterionContext {
    /// The criterion's `conditions` as owned JSON (`"null"` when absent). The game's test of
    /// its `player` predicate follows the mod's test, as vanilla's `SimpleInstance.player`.
    fn conditions_json(&self) -> String;

    /// Whether `stack` passes the item predicate under `key` in the conditions, as the game's
    /// own triggers read an `ItemPredicate`: `items` (an id, a list or a `#tag`), `count`, the
    /// `potion_contents` and `damage` components, and the `enchantments` and `damage`
    /// predicates; other components and predicates aren't checked. A missing key passes, as
    /// an absent `Optional<ItemPredicate>` does. Tag lookups use the target player's world.
    fn item(&self, key: &str, stack: &ItemStack) -> bool;
}

/// A mod's predicate for its advancement trigger, supplied as an `Arc<dyn CriterionTest>` to
/// [`PlayerRef::trigger_criterion`](crate::PlayerRef::trigger_criterion).
///
/// The game calls it synchronously for each open criterion, before testing the criterion's
/// `player` predicate. It keeps neither the test nor its context after the trigger returns.
/// The native and wasm interfaces are generated from this trait, not separate implementations.
/// Wasm predicates run synchronously through a scoped loan of the triggering import's
/// store, including in an already running group. They share the outer call's execution
/// budget and memory limit; the context expires when the predicate returns.
pub trait CriterionTest: Send + Sync + 'static {
    /// Whether the mod's part of this criterion passes. A callback that cannot run refuses
    /// with a typed error, rather than silently answering that the criterion did not match.
    fn test(&self, criterion: &dyn CriterionContext) -> Result<bool, CriterionTestError>;
}

/// Why the mod's criterion test could not answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum CriterionTestError {
    /// The test could not run (including a stopped, busy or unscoped reentrant wasm group).
    Unavailable,
}

impl From<(crate::Identifier, String)> for CriterionTestError {
    fn from(_: (crate::Identifier, String)) -> Self {
        Self::Unavailable
    }
}

/// Why [`PlayerRef::trigger_criterion`](crate::PlayerRef::trigger_criterion) was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum TriggerCriterionError {
    /// The general criterion operation refuses vanilla's `minecraft` triggers,
    /// or the id has an empty namespace or path.
    InvalidTrigger,
    /// This game is a remote client's copy, not the authoritative server.
    NotServer,
    /// The player handle is stale, offline, or displaced by a nested player's turn.
    PlayerUnavailable,
    /// The block position is in a chunk not loaded in the player's level.
    UnloadedChunk,
    /// The generated item boundary refused a supplied stack before firing either trigger.
    Stack(crate::StackWriteError),
    /// A criterion test could not answer. Earlier matching criteria may already be granted.
    Test(CriterionTestError),
    /// The host has no game backend (for example, a mod's test double).
    NoBackend,
}

impl From<crate::StackWriteError> for TriggerCriterionError {
    fn from(error: crate::StackWriteError) -> Self {
        Self::Stack(error)
    }
}

impl NoBackend for TriggerCriterionError {
    fn no_backend() -> Self {
        Self::NoBackend
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EntityRef, Host, Identifier, PlayerRef, ServerHost, WorldEpoch};
    use std::sync::Arc;

    #[test]
    fn a_trigger_without_a_game_refuses_without_testing() {
        struct TestHost;
        impl Host for TestHost {}
        mod_api::impl_host_areas!(TestHost);
        impl ServerHost for TestHost {}
        mod_api::impl_server_host_areas!(TestHost);
        struct Never;
        impl CriterionTest for Never {
            fn test(&self, _: &dyn CriterionContext) -> Result<bool, CriterionTestError> {
                panic!("a refused trigger must not call the mod's predicate")
            }
        }
        let player = PlayerRef {
            entity: EntityRef::new(WorldEpoch::new(1), 1),
            uuid: 1,
        };
        assert_eq!(
            player.trigger_criterion(
                &mut TestHost,
                &Identifier::parse("test:probe").unwrap(),
                Arc::new(Never),
            ),
            Err(TriggerCriterionError::NoBackend)
        );
    }
}
