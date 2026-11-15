use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// The lines of one enabled F3 debug entry a mod registered
    /// (`native_register_debug_entries`), asked each frame the overlay draws
    /// it: push them to `lines`. A disabled entry is never asked.
    pub(in crate::game) fn native_debug_entry_lines(&self, id: &str, lines: &mut Vec<String>) {
        self.fire_debug_entry_lines(id, lines);
    }
}
