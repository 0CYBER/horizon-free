use super::*;

impl Game<'_> {
    /// The atlas sprite source types a mod adds (NeoForge's `RegisterSpriteSourcesEvent`): push
    /// each id, `<mod id>:<path>`. Asked once at start on the client, before resources load.
    pub(in crate::game) fn native_register_sprite_sources(
        context: &mod_api::RegistrationContext,
        sources: &mut Vec<String>,
    ) {
        Self::fire_register_sprite_sources(context, sources);
    }
}
