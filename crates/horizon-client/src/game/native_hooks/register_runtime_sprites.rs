use super::*;

impl Game<'_> {
    pub(in crate::game) fn native_register_runtime_sprites(
        context: &mod_api::RegistrationContext,
        sprites: &mut Vec<RuntimeSprite>,
    ) {
        Self::fire_register_runtime_sprites(context, sprites);
    }
}
