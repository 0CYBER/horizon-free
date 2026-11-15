//! The `render_tooltip_pre` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `RenderTooltipEvent.Pre`: a tooltip, laid out and about to be placed and drawn. `place`
    /// holds the point it hangs from in GUI pixels (the cursor, or the point the screen gives
    /// it), before the tooltip positioner adds its offset and keeps the box on screen; set `x`
    /// and `y` to move it. `screen_width` and `screen_height` are the screen's size. `elements` are its text lines, then its images, as `render_tooltip_gather_components`
    /// left them (a tooltip with no item has only lines), and are read-only here. `stack` is the
    /// item the tooltip describes, `None` for a text tooltip. Set `cancel` and nothing is drawn,
    /// and no `render_tooltip_texture` is asked. Asked each frame a tooltip shows, only while a mod listens.
    /// `draw` is the tooltip's GUI drawing context. Custom drawing stays visible even when
    /// the tooltip is cancelled; `draw.gui_draw_item_with` draws a stack with optional count text.
    /// Client-side.
    #[hook(
        side = "client",
        contract = "cancel",
        out = "cancel",
        neoforge = "RenderTooltipEvent.Pre",
        cancellable = true
    )]
    fn render_tooltip_pre(
        draw: &mut dyn DrawContext,
        stack: Option<&ItemStack>,
        screen_width: i32,
        screen_height: i32,
        elements: &[TooltipElement],
        place: &mut TooltipPlace,
        cancel: &mut bool,
    );
}
