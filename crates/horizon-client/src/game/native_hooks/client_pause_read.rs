use super::*;

/// Helpers for native mods, kept as stable as the hook points.
// Native mods use these; nothing in Horizon does.
#[allow(dead_code)]
impl Game<'_> {
    /// The applied client pause state, not whether a pause screen is open.
    pub(in crate::game) fn is_paused(&self) -> bool {
        self.pause
    }
}
