use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// The world-space debug renderers a mod has now (NeoForge's
    /// `RegisterDebugRenderersEvent`): push a `DebugRenderer` per renderer. Asked at each
    /// renderer list refresh (`Game::refresh_debug_renderers`), on the client.
    pub(in crate::game) fn native_register_debug_renderers(
        context: &mod_api::RegistrationContext,
        renderers: &mut Vec<DebugRenderer>,
    ) {
        Self::fire_register_debug_renderers(context, renderers);
    }
}
