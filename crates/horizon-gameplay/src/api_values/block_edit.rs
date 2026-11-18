//! Cold, all-or-nothing preparation of typed bulk edit values.
use super::{TryIntoApi, TryIntoEngine, block_metadata_table};
use horizon_command::args::BlockInput;
use horizon_world::registry::BlockStateId;
use mod_api::BlockMetadata;

pub struct PreparedBlockEdit {
    pub palette: Vec<u32>,
    pub filters: Vec<BlockInput>,
}

/// Resolve every palette state and compile every filter before a world mutation is possible.
pub fn prepare(edit: &mod_api::BlockEdit) -> Result<PreparedBlockEdit, String> {
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
pub fn compile_filter(predicate: &mod_api::StatePredicate) -> Result<BlockInput, String> {
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
pub fn column_palette(palette: Vec<BlockStateId>) -> Result<Vec<mod_api::BlockState>, String> {
    palette
        .try_into_api()
        .map_err(|error| format!("column palette: {error:?}"))
}
