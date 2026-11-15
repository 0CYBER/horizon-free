use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// Whether a mod hides `state` from the mesher (`hidden` starts false); asked once for every
    /// state at join and on `refresh_render_visibility`. Client-side.
    pub(in crate::game) fn native_block_render_visibility(
        &self,
        state: BlockStateId,
        hidden: &mut bool,
    ) {
        self.fire_block_render_visibility(state, hidden);
    }
}
