//! Typed partial block predicates for prepared bulk edits.
use crate::{BlockKey, BlockMetadata, BlockState, BlockStateError, StateConstraint, TagKey, Tags};

/// A block or tag filter, with unspecified properties left as wildcards.
/// An unknown or missing property on a tag member never matches.
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct StatePredicate {
    target: StatePredicateTarget,
    constraints: Vec<StateConstraint>,
}

/// Derived transport selector, not an engine predicate.
// exhaustive: the closed serialized block/tag selector vocabulary is a link contract.
// hidden: selector used by the derived predicate codec, not a second public constructor.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
pub enum StatePredicateTarget {
    Block(BlockKey),
    Tag(TagKey),
}
/// Derived transport. The game validates its constraints before compiling any edit.
// hidden: generated codec transport for the private predicate fields.
#[doc(hidden)]
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct StatePredicateRecord {
    pub target: StatePredicateTarget,
    pub constraints: Vec<StateConstraint>,
}
impl StatePredicateRecord {
    /// Construct the derived transport record.
    pub fn new(target: StatePredicateTarget, constraints: Vec<StateConstraint>) -> Self {
        Self {
            target,
            constraints,
        }
    }
}
impl StatePredicate {
    /// A block filter; unknown properties, illegal values and duplicate properties refuse.
    /// A partial filter need not produce the default state's complete combination.
    pub fn block(
        metadata: &dyn BlockMetadata,
        block: BlockKey,
        constraints: &[StateConstraint],
    ) -> Result<Self, BlockStateError> {
        let target = StatePredicateTarget::Block(block);
        validate(metadata, target, constraints)?;
        Ok(Self {
            target,
            constraints: constraints.to_vec(),
        })
    }
    /// A tag filter; duplicate properties refuse, but arbitrary properties and empty tags remain valid.
    /// Membership is evaluated against authoritative tags at application, not frozen here.
    pub fn tag(
        metadata: &dyn BlockMetadata,
        _tags: &dyn Tags,
        tag: TagKey,
        constraints: &[StateConstraint],
    ) -> Result<Self, BlockStateError> {
        let target = StatePredicateTarget::Tag(tag);
        validate(metadata, target, constraints)?;
        Ok(Self {
            target,
            constraints: constraints.to_vec(),
        })
    }
    /// The exact block this predicate selects, if it is not a tag filter.
    pub fn block_key(&self) -> Option<BlockKey> {
        match self.target {
            StatePredicateTarget::Block(key) => Some(key),
            _ => None,
        }
    }
    /// The tag this predicate selects, if it is not an exact block filter.
    pub fn tag_key(&self) -> Option<TagKey> {
        match self.target {
            StatePredicateTarget::Tag(key) => Some(key),
            _ => None,
        }
    }
    /// Explicit property requirements; all must match, but omitted properties are unrestricted.
    pub fn constraints(&self) -> &[StateConstraint] {
        &self.constraints
    }
}
fn validate(
    metadata: &dyn BlockMetadata,
    target: StatePredicateTarget,
    constraints: &[StateConstraint],
) -> Result<(), BlockStateError> {
    let default = match target {
        StatePredicateTarget::Block(block) => Some(BlockState::of(metadata, block, &[])?),
        StatePredicateTarget::Tag(_) => None,
    };
    for (input, constraint) in constraints.iter().enumerate() {
        if constraints[..input]
            .iter()
            .any(|previous| previous.name == constraint.name)
        {
            return Err(BlockStateError::DuplicateProperty(input as u32));
        }
        if let Some(default) = default {
            match default.with(metadata, &constraint.name, &constraint.value) {
                // This is schema validation, not filling unspecified fields from defaults.
                // A legal partial property can match a nondefault combination even if this
                // particular complete default-plus-one combination does not exist.
                Ok(_) | Err(BlockStateError::InvalidCombination) => {}
                Err(BlockStateError::UnknownProperty(_)) => {
                    return Err(BlockStateError::UnknownProperty(input as u32));
                }
                Err(BlockStateError::InvalidValue(_)) => {
                    return Err(BlockStateError::InvalidValue(input as u32));
                }
                Err(error) => return Err(error),
            }
        }
    }
    Ok(())
}
/// Validate unknown decoded transport at the owning game boundary before mutation.
// hidden: owning-boundary validation, exported only through __private.
#[doc(hidden)]
pub fn validate_state_predicate(
    metadata: &dyn BlockMetadata,
    predicate: &StatePredicate,
) -> Result<(), BlockStateError> {
    validate(metadata, predicate.target, &predicate.constraints)
}
impl From<StatePredicateRecord> for StatePredicate {
    fn from(value: StatePredicateRecord) -> Self {
        Self {
            target: value.target,
            constraints: value.constraints,
        }
    }
}
impl From<&StatePredicate> for StatePredicateRecord {
    fn from(value: &StatePredicate) -> Self {
        Self {
            target: value.target,
            constraints: value.constraints.clone(),
        }
    }
}

#[cfg(test)]
#[path = "state_predicate/tests.rs"]
mod tests;
