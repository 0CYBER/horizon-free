use super::*;

impl Game<'_> {
    /// Collect the mods' render pipeline registrations once at startup.
    pub(in crate::game) fn native_register_render_pipelines(
        context: &mod_api::RegistrationContext,
        pipelines: &mut Vec<mod_api::RenderPipeline>,
    ) {
        Self::fire_register_render_pipelines(context, pipelines);
    }
}
