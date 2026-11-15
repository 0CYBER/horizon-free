use super::*;

impl Game<'_> {
    /// The block-state definition types a mod adds (NeoForge's `RegisterBlockStateModels`): push
    /// each id, `<mod id>:<path>`. Asked once at start on the client, before resources load.
    pub(in crate::game) fn native_register_block_state_models(
        context: &mod_api::RegistrationContext,
        definitions: &mut Vec<String>,
    ) {
        Self::fire_register_block_state_models(context, definitions);
    }
}
