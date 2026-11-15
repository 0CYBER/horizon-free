use super::*;

impl Game<'_> {
    /// Collect typed client loading-screen registrations once at startup.
    pub(in crate::game) fn native_register_dimension_transition_screen(
        context: &mod_api::RegistrationContext,
        screens: &mut Vec<mod_api::DimensionTransitionScreen>,
    ) {
        Self::fire_register_dimension_transition_screen(context, screens);
    }
}
