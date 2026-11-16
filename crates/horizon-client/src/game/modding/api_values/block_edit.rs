//! Cold, all-or-nothing preparation of typed bulk edit values.
use super::{TryIntoApi, TryIntoEngine, block_metadata_table};
use horizon_command::args::BlockInput;
use horizon_world::registry::BlockStateId;
use mod_api::BlockMetadata;

pub(in crate::game) struct PreparedBlockEdit {
    pub(in crate::game) palette: Vec<u32>,
    pub(in crate::game) filters: Vec<BlockInput>,
}

/// Resolve every palette state and compile every filter before a world mutation is possible.
pub(in crate::game) fn prepare(edit: &mod_api::BlockEdit) -> Result<PreparedBlockEdit, String> {
    if edit.filters.len() > 65_536 {
        return Err("more than 65536 filters".into());
    }
    let palette = edit
        .palette
        .iter()
        .enumerate()
        .map(|(index, state)| {
            let state: BlockStateId = state
                .try_into_engine(&())
                .map_err(|error| format!("palette entry {index}: {error:?}"))?;
            Ok(state.0)
        })
        .collect::<Result<Vec<_>, String>>()?;
    let filters = edit
        .filters
        .iter()
        .enumerate()
        .map(|(index, predicate)| {
            compile_filter(predicate).map_err(|error| format!("filter {index}: {error}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(PreparedBlockEdit { palette, filters })
}

/// Compile into the existing command matcher; do not create a second predicate algorithm.
fn compile_filter(predicate: &mod_api::StatePredicate) -> Result<BlockInput, String> {
    let metadata = block_metadata_table().map_err(|error| format!("block metadata: {error:?}"))?;
    mod_api::__private::validate_state_predicate(metadata, predicate)
        .map_err(|error| format!("predicate: {error:?}"))?;
    let record = mod_api::StatePredicateRecord::from(predicate);
    let (id, tag) = match record.target {
        mod_api::StatePredicateTarget::Block(key) => (
            metadata
                .name(key)
                .map_err(|error| format!("block: {error:?}"))?
                .to_owned(),
            false,
        ),
        mod_api::StatePredicateTarget::Tag(key) => (
            mod_api::__private::with_tag_name(key, |name| name.map(str::to_owned))
                .ok_or("block tag is unavailable")?,
            true,
        ),
    };
    Ok(BlockInput {
        id,
        properties: record
            .constraints
            .into_iter()
            .map(|constraint| (constraint.name, constraint.value))
            .collect(),
        tag,
        nbt: None,
    })
}

/// Convert only the distinct numeric palette entries, preserving complete property states.
pub(in crate::game) fn column_palette(
    palette: Vec<BlockStateId>,
) -> Result<Vec<mod_api::BlockState>, String> {
    palette
        .try_into_api()
        .map_err(|error| format!("column palette: {error:?}"))
}

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
