use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// The numeric item model properties a mod adds (NeoForge's
    /// `RegisterRangeSelectItemModelPropertyEvent`): push each id, `<mod id>:<path>`.
    /// Asked once at start on the client, before resources load.
    pub(in crate::game) fn native_register_range_select_item_model_property(
        context: &mod_api::RegistrationContext,
        properties: &mut Vec<String>,
    ) {
        Self::fire_register_range_select_item_model_property(context, properties);
    }
}
