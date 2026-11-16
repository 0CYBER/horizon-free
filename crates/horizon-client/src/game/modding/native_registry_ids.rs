//! Network ids of registry entries for mods (`registry_protocol_id`).
use super::Game;
use horizon_core::identifier::Identifier;
use horizon_protocol::registry::SyncedNames;

impl Game<'_> {
    /// The network id of `name` in `registry`: a client's own connection's registries (never the
    /// integrated server's), a server's synchronized ones, as its connections are sent them.
    /// None with no connection.
    pub(in crate::game) fn registry_protocol_id(
        &self,
        registry: &Identifier,
        name: &Identifier,
    ) -> Option<i32> {
        if let Some(remote) = self.remote.as_ref() {
            return SyncedNames {
                builtin: &remote.builtin,
                dynamic: &remote.registries,
            }
            .id(registry, name);
        }
        self.lan.as_ref()?.hub.synced_id(registry, name)
    }
}
