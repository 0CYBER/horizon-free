//! Cold, all-or-nothing preparation of typed bulk edit values.
//! The preparation lives in horizon-gameplay; this module keeps its tests.
#[cfg(test)]
use super::{TryIntoApi, block_metadata_table};
#[cfg(test)]
use horizon_command::args::BlockInput;
pub(in crate::game) use horizon_gameplay::api_values::block_edit::*;
#[cfg(test)]
use horizon_world::registry::BlockStateId;
#[cfg(test)]
mod tests {
    use super::*;
    use horizon_world::registry::BlockRegistry;
    use mod_api::{StateConstraint, StatePredicate, StatePredicateRecord, StatePredicateTarget};
    use std::collections::{HashMap, HashSet};

    fn registry() -> BlockRegistry {
        mod_platform::__private::part_tests::install_parts_fixture(&[]).unwrap();
        let facts = r#"{"defaults":{"minecraft:air":0,"minecraft:stairs":1},"states":[["a",true,false,"","minecraft:air"],["s",false,false,"",{"id":"minecraft:stairs","properties":{"left":"false","right":"false"}}],["s",false,false,"",{"id":"minecraft:stairs","properties":{"left":"true","right":"true"}}]]}"#;
        let registry = BlockRegistry::from_facts_json(facts.as_bytes()).unwrap();
        super::super::resolve_block_definitions(&registry).unwrap();
        registry
    }
    struct Tags;
    impl mod_api::Tags for Tags {}

    #[test]
    fn typed_bulk_sparse_predicates_use_existing_matcher_and_unknown_tag_properties_never_match() {
        let registry = registry();
        let metadata = block_metadata_table().unwrap();
        let state = BlockStateId(2).try_into_api().unwrap();
        let exact = StatePredicate::block(
            metadata,
            state.block(),
            &[StateConstraint::new("left", "true")],
        )
        .unwrap();
        let filter = compile_filter(&exact).unwrap();
        let tags = HashMap::from([(
            "stairs".to_owned(),
            HashSet::from(["minecraft:stairs".to_owned()]),
        )]);
        let matches = |filter: &BlockInput, id| {
            crate::game::commands::block_input_matches(
                &registry,
                [&tags, &tags],
                filter,
                BlockStateId(id),
            )
        };
        assert!(matches(&filter, 2));
        assert!(!matches(&filter, 1));
        let wildcard =
            compile_filter(&StatePredicate::block(metadata, state.block(), &[]).unwrap()).unwrap();
        assert!(matches(&wildcard, 1));
        assert!(matches(&wildcard, 2));
        let tag =
            mod_api::__private::tag_key(&mod_api::Identifier::parse("minecraft:stairs").unwrap())
                .unwrap();
        let tag_filter = |constraints: &[StateConstraint]| {
            compile_filter(&StatePredicate::tag(metadata, &Tags, tag, constraints).unwrap())
                .unwrap()
        };
        assert!(matches(&tag_filter(&[]), 2));
        assert!(!matches(&tag_filter(&[]), 0));
        // ASSERTION mutant: discarding unknown tag constraints would make this match.
        assert!(!matches(
            &tag_filter(&[StateConstraint::new("unknown", "anything")]),
            2
        ));
        assert!(!matches(
            &tag_filter(&[StateConstraint::new("left", "invalid")]),
            2
        ));
        let empty = mod_api::__private::tag_key(&mod_api::Identifier::parse("test:empty").unwrap())
            .unwrap();
        let empty = StatePredicate::tag(
            metadata,
            &Tags,
            empty,
            &[StateConstraint::new("unknown", "anything")],
        )
        .unwrap();
        assert!(!matches(&compile_filter(&empty).unwrap(), 2));
    }

    #[test]
    fn typed_bulk_preparation_rejects_late_bad_values_before_any_write() {
        let _registry = registry();
        let state = BlockStateId(2).try_into_api().unwrap();
        let good =
            StatePredicate::block(block_metadata_table().unwrap(), state.block(), &[]).unwrap();
        let bad = StatePredicate::from(StatePredicateRecord::new(
            StatePredicateTarget::Block(state.block()),
            vec![StateConstraint::new("unknown", "true")],
        ));
        let mut edit = mod_api::BlockEdit {
            palette: vec![state],
            filters: vec![good, bad],
            runs: vec![],
        };
        assert!(
            prepare(&edit).is_err(),
            "late bad filter must refuse the whole prepared edit"
        );
        edit.filters.pop();
        let prepared = prepare(&edit).unwrap();
        assert_eq!(prepared.palette, [2]);
        assert_eq!(prepared.filters.len(), 1);
        edit.palette
            .push(mod_api::BlockStateRecord::new(state.block(), u16::MAX).into());
        assert!(
            prepare(&edit).is_err(),
            "late bad palette state must refuse the whole prepared edit"
        );
        let palette = column_palette(vec![BlockStateId(1), BlockStateId(2)]).unwrap();
        assert_eq!(
            palette
                .iter()
                .map(|state| state.ordinal())
                .collect::<Vec<_>>(),
            [0, 1]
        );
        assert_eq!(
            palette[0]
                .property(block_metadata_table().unwrap(), "left")
                .unwrap(),
            Some("false")
        );
        assert_eq!(
            palette[1]
                .property(block_metadata_table().unwrap(), "right")
                .unwrap(),
            Some("true")
        );
    }
}
