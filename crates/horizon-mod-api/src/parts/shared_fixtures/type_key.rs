#[cfg(test)]
pub(crate) fn type_key(id: &str) -> BlockEntityTypeKey {
    mod_vocab::__private::parts_installation::requested_block_entity_type_key(id)
        .expect("fixture requested this type")
}
