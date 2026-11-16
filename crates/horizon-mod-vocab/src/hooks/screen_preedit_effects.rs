//! The `screen_preedit_effects` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Before a screen's preedit handler (`ScreenEvent.Preedit.Pre`, vanilla's
    /// `GuiEventListener.preeditUpdated`): the input method is composing
    /// `full_text` (empty when the composition ended), with the caret at
    /// byte `caret_position`, split into `blocks` of which `focused_block` is
    /// the one being converted. Set `canceled` and no text field sees it, and
    /// `screen_preedit_post` does not fire. Fires on the window while a
    /// screen is open, even when no text field is focused.
    #[hook(
        side = "client",
        contract = "cancel",
        out = "canceled",
        neoforge = "ScreenEvent.Preedit.Pre",
        cancellable = true
    )]
    fn screen_preedit_pre();
    /// After a screen's preedit handler (`ScreenEvent.Preedit.Post`) and only
    /// when no text field took the composition (the handler returned false:
    /// vanilla's focused field takes it, so this is a preedit no field
    /// wanted) and `screen_preedit_pre` did not cancel. Same arguments.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "ScreenEvent.Preedit.Post",
        cancellable = false
    )]
    fn screen_preedit_post();
    /// Ahead of the effect panels beside the survival and creative
    /// inventories (`ScreenEvent.RenderInventoryMobEffects`, vanilla's
    /// `EffectsInInventory.extractRenderState`): `available_space` is the
    /// width in GUI pixels right of the container, at least 32 (with no effect
    /// to show, or less room, vanilla draws nothing and this does not fire).
    /// `compact` picks icons over full panels (vanilla's choice is compact
    /// under 120) and `horizontal_offset` shifts the stack right (negative
    /// left). Set `canceled` and the panels are not drawn: draw your own with
    /// `screen_render_post`. Once a frame while the screen draws them,
    /// on the window only; if the effects space changes within a frame (the
    /// recipe book opening between two layout builds), it is asked again for
    /// the new space. The exclusion areas follow this choice in the same frame.
    #[hook(
        side = "client",
        contract = "cancel",
        out = "canceled",
        neoforge = "ScreenEvent.RenderInventoryMobEffects",
        cancellable = true
    )]
    fn screen_render_inventory_mob_effects();
}
