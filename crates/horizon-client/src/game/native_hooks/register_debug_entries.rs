use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// The F3 debug entries a mod adds (NeoForge's `RegisterDebugEntriesEvent`):
    /// push a `DebugEntry` per entry, its id `<mod id>:<path>` and the vanilla
    /// group its lines join. Asked once at start, on the client.
    pub(in crate::game) fn native_register_debug_entries(
        context: &mod_api::RegistrationContext,
        entries: &mut Vec<DebugEntry>,
    ) {
        Self::fire_register_debug_entries(context, entries);
    }
}
