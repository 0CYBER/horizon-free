use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// A physical key or mouse button bound to the gameplay mapping `mapping` (`key.jump`) changes
    /// state while playing, before the game acts on it (`pressed` for a press, repeats included,
    /// and not for a release). Set `canceled` on a press to drop it for every mapping bound to its
    /// input; `canceled` on a release is ignored, so the game always lets go of what it held.
    pub(in crate::game) fn native_key_mapping_pre(
        &mut self,
        mapping: &str,
        pressed: bool,
        canceled: &mut bool,
    ) {
        self.fire_key_mapping_pre(mapping, pressed, canceled);
    }
}
