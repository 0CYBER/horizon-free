use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// `IItemExtension.getHighlightTip`: the name drawn above the hotbar for
    /// the selected `stack`. `name` arrives with vanilla's styled hover name;
    /// the parts of `stack`'s item may replace it. Client-side.
    #[inline(never)]
    pub(in crate::game) fn native_highlight_tip(&self, stack: &ItemStack, name: &mut Text) {
        self.fire_highlight_tip(stack, name);
    }
}
