//! Cold registration construction, with native representation kept private.
/// Construct or inspect the owned registration value.
pub fn block_entity_declaration(id: String, blocks: Vec<String>) -> crate::BlockEntityTypeDecl {
    crate::BlockEntityTypeDecl { id, blocks }
}
