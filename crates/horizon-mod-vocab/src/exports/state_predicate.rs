mod state_predicate;
pub use state_predicate::{StatePredicate, StatePredicateRecord, StatePredicateTarget};
pub mod __private {
    pub use crate::state_predicate::validate_state_predicate;
}
