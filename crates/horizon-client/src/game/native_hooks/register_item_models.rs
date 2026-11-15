use super::*;

impl Game<'_> {
    /// The item model node types a mod adds (NeoForge's `RegisterItemModelsEvent`): push each
    /// id, `<mod id>:<path>`. Asked once at start on the client, before resources load.
    pub(in crate::game) fn native_register_item_models(
        context: &mod_api::RegistrationContext,
        models: &mut Vec<String>,
    ) {
        Self::fire_register_item_models(context, models);
    }
}
