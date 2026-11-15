use super::*;

impl Game<'_> {
    /// The categorical item model properties a mod adds (NeoForge's
    /// `RegisterSelectItemModelPropertyEvent`): push each id, `<mod id>:<path>`.
    /// Asked once at start on the client, before resources load.
    pub(in crate::game) fn native_register_select_item_model_property(
        context: &mod_api::RegistrationContext,
        properties: &mut Vec<String>,
    ) {
        Self::fire_register_select_item_model_property(context, properties);
    }
}
