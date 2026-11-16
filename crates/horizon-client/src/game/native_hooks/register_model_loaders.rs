use super::*;

impl Game<'_> {
    /// The model loaders a mod adds (NeoForge's `ModelEvent.RegisterLoaders`): push each id,
    /// `<mod id>:<path>`. Asked once at start on the client, before resources load.
    pub(in crate::game) fn native_register_model_loaders(
        context: &mod_api::RegistrationContext,
        loaders: &mut Vec<String>,
    ) {
        Self::fire_register_model_loaders(context, loaders);
    }
}
