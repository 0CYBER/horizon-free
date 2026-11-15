//! Owned inputs for testing the engine's tag merge without publishing mutable lookup tables.
/// Merge only block rows and return the selected block's owned metadata view.
#[cfg(any(test, feature = "testing"))]
pub fn merged_block_rows(
    keys: super::PartsInput,
    tags: super::TagPartsInput,
    bindings: &std::collections::HashMap<String, std::collections::HashSet<String>>,
    id: &str,
) -> Vec<super::AttachedInput<dyn crate::BlockPart>> {
    let mut keys: crate::parts::Parts = keys.into();
    let tags: crate::parts::TagParts = tags.into();
    crate::parts::add_tagged(&mut keys.blocks, &tags.blocks, bindings, "block");
    keys.blocks
        .get(id)
        .into_iter()
        .flatten()
        .map(Into::into)
        .collect()
}
