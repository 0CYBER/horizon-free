use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// Register server data reload listeners after packs are selected. The
    /// host gives registry access at registration, as
    /// `AddServerReloadListenersEvent.getRegistryAccess` does.
    pub(in crate::game) fn native_add_server_reload_listeners(
        &self,
        listeners: &mut dyn ServerReloadListeners,
    ) {
        self.fire_add_server_reload_listeners(listeners);
    }
}
