use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// Whether `fluid`'s side beside `state` uses the overlay sprite (`overlay` starts with
    /// vanilla's `HalfTransparentBlock`/`LeavesBlock` test); asked for every state and fluid
    /// where `refresh_render_visibility` resolves its tables. Client-side.
    pub(in crate::game) fn native_should_display_fluid_overlay(
        &self,
        state: BlockStateId,
        fluid: mod_api::FluidState,
        overlay: &mut bool,
    ) {
        self.fire_should_display_fluid_overlay(state, fluid, overlay);
    }
}
