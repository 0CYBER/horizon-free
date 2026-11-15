use super::*;

impl Game<'_> {
    /// The item tint sources a mod adds (NeoForge's `RegisterColorHandlersEvent.ItemTintSources`):
    /// push each id, `<mod id>:<path>`. Asked once at start on the client, before resources load.
    pub(in crate::game) fn native_register_item_tint_sources(
        context: &mod_api::RegistrationContext,
        sources: &mut Vec<String>,
    ) {
        Self::fire_register_item_tint_sources(context, sources);
    }
}
