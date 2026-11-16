use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// Ahead of the effect panels beside the survival and creative
    /// inventories (`ScreenEvent.RenderInventoryMobEffects`, vanilla's
    /// `EffectsInInventory.extractRenderState`): `available_space` is the
    /// width in GUI pixels right of the container, at least 32 (with no effect
    /// to show, or less room, vanilla draws nothing and this does not fire).
    /// `compact` picks icons over full panels (vanilla's choice is compact
    /// under 120) and `horizontal_offset` shifts the stack right (negative
    /// left). Set `canceled` and the panels are not drawn: draw your own with
    /// `native_screen_render_post`. Once a frame while the screen draws them,
    /// on the window only; if the effects space changes within a frame (the
    /// recipe book opening between two layout builds), it is asked again for
    /// the new space. The exclusion areas follow this choice in the same frame.
    pub(in crate::game) fn native_screen_render_inventory_mob_effects(
        &self,
        screen: &ScreenView<'_>,
        available_space: i32,
        compact: &mut bool,
        horizontal_offset: &mut i32,
        canceled: &mut bool,
    ) {
        self.fire_screen_render_inventory_mob_effects(
            screen,
            available_space,
            compact,
            horizontal_offset,
            canceled,
        );
    }
    /// Before a screen's preedit handler (`ScreenEvent.Preedit.Pre`, vanilla's
    /// `GuiEventListener.preeditUpdated`): the input method is composing
    /// `full_text` (empty when the composition ended), with the caret at
    /// byte `caret_position`, split into `blocks` of which `focused_block` is
    /// the one being converted. Set `canceled` and no text field sees it, and
    /// `native_screen_preedit_post` does not fire. Fires on the window while a
    /// screen is open, even when no text field is focused.
    pub(in crate::game) fn native_screen_preedit_pre(
        &self,
        screen: &ScreenView<'_>,
        full_text: &str,
        caret_position: usize,
        blocks: &[String],
        focused_block: usize,
        canceled: &mut bool,
    ) {
        self.fire_screen_preedit_pre(
            screen,
            full_text,
            caret_position,
            blocks,
            focused_block,
            canceled,
        );
    }
    /// After a screen's preedit handler (`ScreenEvent.Preedit.Post`) and only
    /// when no text field took the composition (the handler returned false:
    /// vanilla's focused field takes it, so this is a preedit no field
    /// wanted) and `native_screen_preedit_pre` did not cancel. Same arguments.
    pub(in crate::game) fn native_screen_preedit_post(
        &self,
        screen: &ScreenView<'_>,
        full_text: &str,
        caret_position: usize,
        blocks: &[String],
        focused_block: usize,
    ) {
        self.fire_screen_preedit_post(screen, full_text, caret_position, blocks, focused_block);
    }
}
