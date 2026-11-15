use super::*;
use mod_api::LoadConditions;
impl Game<'_> {
    pub(in crate::game) fn native_register_load_conditions(
        context: &mod_api::RegistrationContext,
        conditions: &mut dyn LoadConditions,
    ) {
        Self::fire_register_load_conditions(context, conditions);
    }
}
