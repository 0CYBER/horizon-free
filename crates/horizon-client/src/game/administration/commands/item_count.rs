//! The `count` wrapper of an item predicate (`ItemPredicateArgument`): both
//! `count=` and `count~` read the live stack count.

use horizon_core::nbt::Tag;

/// `MinMaxBounds.Ints.matches` on the stack count: an exact numeric tag, or a
/// compound with optional numeric `min` and `max` (`min <= max`).
pub(super) fn matches(count: impl Into<i64>, want: &Tag) -> bool {
    super::item_predicate::matches_int_bounds(count.into(), want)
}
