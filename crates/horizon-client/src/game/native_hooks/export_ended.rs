use super::*;

impl Game<'_> {
    /// Tells client mods when a mod-started export has settled.
    pub(in crate::game) fn native_export_ended(
        &mut self,
        request: u32,
        state: ExportState,
        frames: u32,
    ) {
        self.fire_export_ended(request, state, frames);
    }
}
