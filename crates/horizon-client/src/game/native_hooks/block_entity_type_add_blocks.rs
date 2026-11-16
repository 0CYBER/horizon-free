use super::*;
impl Game<'_> {
    pub(in crate::game) fn native_block_entity_type_add_blocks(
        context: &mod_api::RegistrationContext,
        types: &[mod_api::BlockEntityTypeKey],
        additions: &mut Vec<(mod_api::BlockEntityTypeKey, mod_api::BlockKey)>,
    ) {
        Self::fire_block_entity_type_add_blocks(context, types, additions);
    }
}
