use super::*;
use std::collections::BTreeMap;

fn metadata() -> horizon_mod_runtime::block_definitions::Blocks {
    let off = BTreeMap::from([
        ("left".into(), "false".into()),
        ("right".into(), "false".into()),
    ]);
    let on = BTreeMap::from([
        ("left".into(), "true".into()),
        ("right".into(), "true".into()),
    ]);
    horizon_mod_runtime::block_definitions::Blocks {
        names: Default::default(),
        definitions: vec![
            horizon_mod_runtime::block_definitions::BlockDefinition::new(&[&off, &on], 0).unwrap(),
        ],
    }
}
struct TagsAvailable;
impl Tags for TagsAvailable {}

#[test]
fn strict_block_predicates_validate_schema_without_default_bias() {
    let metadata = metadata();
    let key = BlockKey(0);
    // The default is false/false, so setting just left=true would not be a complete state.
    // It is nevertheless a legal partial predicate matching the independent true/true row.
    assert!(StatePredicate::block(&metadata, key, &[StateConstraint::new("left", "true")]).is_ok());
    assert_eq!(
        StatePredicate::block(&metadata, key, &[StateConstraint::new("missing", "true")])
            .unwrap_err(),
        BlockStateError::UnknownProperty(0)
    );
    assert_eq!(
        StatePredicate::block(
            &metadata,
            key,
            &[StateConstraint::new("left", "not_a_value")]
        )
        .unwrap_err(),
        BlockStateError::InvalidValue(0)
    );
    assert_eq!(
        StatePredicate::block(
            &metadata,
            key,
            &[
                StateConstraint::new("left", "true"),
                StateConstraint::new("left", "false")
            ]
        )
        .unwrap_err(),
        BlockStateError::DuplicateProperty(1)
    );
    for (name, value, error) in [
        ("missing", "true", BlockStateError::UnknownProperty(1)),
        ("right", "invalid", BlockStateError::InvalidValue(1)),
    ] {
        assert_eq!(
            StatePredicate::block(
                &metadata,
                key,
                &[
                    StateConstraint::new("left", "true"),
                    StateConstraint::new(name, value)
                ]
            )
            .unwrap_err(),
            error
        );
    }
    let decoded = StatePredicate::from(StatePredicateRecord::new(
        StatePredicateTarget::Block(key),
        vec![StateConstraint::new("left", "invalid")],
    ));
    assert_eq!(
        validate_state_predicate(&metadata, &decoded),
        Err(BlockStateError::InvalidValue(0))
    );
}

#[test]
fn tag_predicates_keep_vague_properties_and_refuse_duplicate_transport() {
    let metadata = metadata();
    let vague = StatePredicate::tag(
        &metadata,
        &TagsAvailable,
        TagKey(0),
        &[StateConstraint::new(
            "property_no_member_has",
            "arbitrary_value",
        )],
    )
    .unwrap();
    assert_eq!(vague.constraints()[0].name, "property_no_member_has");
    assert_eq!(vague.tag_key(), Some(TagKey(0)));
    assert_eq!(
        StatePredicate::from(StatePredicateRecord::from(&vague)),
        vague
    );
    let decoded = StatePredicate::from(StatePredicateRecord::new(
        StatePredicateTarget::Tag(TagKey(0)),
        vec![
            StateConstraint::new("unknown", "a"),
            StateConstraint::new("unknown", "b"),
        ],
    ));
    assert_eq!(
        validate_state_predicate(&metadata, &decoded),
        Err(BlockStateError::DuplicateProperty(1))
    );
}
