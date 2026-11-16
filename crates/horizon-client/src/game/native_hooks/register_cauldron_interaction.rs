use super::*;
impl Game<'_> {
    /// Declare cauldron interactions before publishing the resource snapshot.
    pub(in crate::game) fn native_register_cauldron_interaction(
        context: &mod_api::RegistrationContext,
        interactions: &mut dyn CauldronInteractions,
    ) {
        Self::fire_register_cauldron_interaction(context, interactions);
    }
}
