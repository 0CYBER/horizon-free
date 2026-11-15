use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// `FluidTooltipEvent`: the tooltip of `stack`, a fluid the game draws a slot of (the recipe
    /// viewer's fluid wells), once its name, its amount in millibuckets and, with F3+H, its id
    /// are in `lines`, each with a colour as `rgba` packs it (`u32::MAX` keeps the text's own).
    /// Add, change, reorder or remove lines, and an empty list draws nothing. It fires before
    /// the tooltip is laid out or drawn, with no item and no `item_tooltip` involved: a tank's
    /// or a fluid slot's tooltip has neither. Asked each frame the tooltip shows, only while a
    /// mod listens. Client-side.
    pub(in crate::game) fn native_fluid_tooltip(
        &self,
        call: mod_api::__private::FluidTooltipCall<'_>,
    ) {
        self.fire_fluid_tooltip(call);
    }
}
