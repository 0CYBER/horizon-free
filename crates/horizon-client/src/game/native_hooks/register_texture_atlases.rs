use super::*;

impl Game<'_> {
    pub(in crate::game) fn native_register_texture_atlases(
        context: &mod_api::RegistrationContext,
        atlases: &mut Vec<TextureAtlasRegistration>,
    ) {
        Self::fire_register_texture_atlases(context, atlases);
    }
}
