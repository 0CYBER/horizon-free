use super::*;
impl Game<'_> {
    pub(in crate::game) fn native_register_villager_professions(
        context: &mod_api::RegistrationContext,
        professions: &mut dyn VillagerProfessions,
    ) {
        Self::fire_register_villager_professions(context, professions);
    }
}
