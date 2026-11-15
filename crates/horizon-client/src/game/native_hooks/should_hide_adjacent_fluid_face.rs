use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// Whether `state` hides the face `fluid` draws toward it, `direction` pointing from the
    /// fluid to the block (`hidden` starts false); asked for every state and fluid where
    /// `refresh_render_visibility` resolves its tables. Client-side.
    pub(in crate::game) fn native_should_hide_adjacent_fluid_face(
        &self,
        state: BlockStateId,
        direction: Direction,
        fluid: mod_api::FluidState,
        hidden: &mut bool,
    ) {
        self.fire_should_hide_adjacent_fluid_face(state, direction, fluid, hidden);
    }
}
