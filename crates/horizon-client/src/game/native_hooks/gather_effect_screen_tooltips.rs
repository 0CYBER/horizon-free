use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// `GatherEffectScreenTooltipsEvent`: the tooltip of the effect panel the
    /// cursor is over beside the survival or creative inventory. `tooltip`
    /// starts empty when the panel's information is fully visible, or holds
    /// the full name then duration when compact or clipped. Each line has a
    /// colour as `rgba` packs it (`u32::MAX` keeps the text's own); add, change,
    /// reorder or remove lines, and an empty list draws nothing. `effect` is
    /// the effect the panel shows, `screen` the inventory. Asked each frame
    /// a rendered panel is hovered, only while a mod listens. Client-side.
    pub(in crate::game) fn native_gather_effect_screen_tooltips(
        &self,
        screen: &ScreenView<'_>,
        effect: &ActiveEffect,
        tooltip: &mut Vec<(Text, u32)>,
    ) {
        self.fire_gather_effect_screen_tooltips(screen, effect, tooltip);
    }
}
