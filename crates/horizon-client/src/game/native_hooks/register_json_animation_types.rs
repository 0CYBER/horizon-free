use super::*;

impl Game<'_> {
    /// The animation targets and interpolations a mod adds (NeoForge's
    /// `RegisterJsonAnimationTypesEvent`), each with the mod that registered it. Asked once at
    /// start on the client, before resources load.
    pub(in crate::game) fn native_register_json_animation_types(
        context: &mod_api::RegistrationContext,
        types: &mut Vec<Answered<JsonAnimationType>>,
    ) {
        Self::fire_register_json_animation_types(context, types);
    }
}
