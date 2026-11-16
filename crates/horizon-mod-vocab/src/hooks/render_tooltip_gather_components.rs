//! The `render_tooltip_gather_components` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `RenderTooltipEvent.GatherComponents`: the elements of `stack`'s
    /// tooltip (`ItemStack.getTooltipLines` as text lines, then the image
    /// component its item's `tooltip_component` part answered), once
    /// `item_tooltip` has run and before any wrapping or measuring. Add,
    /// change, reorder or remove elements; the lines draw in order, then the
    /// images in order, under any bundle's contents. `max_width` starts at -1,
    /// no limit, which leaves the lines as they are; a mod that sets it
    /// wraps every line wider than that, in GUI pixels. Set `cancel` and
    /// nothing is drawn, and no `render_tooltip_texture` is asked. Asked each frame
    /// the tooltip shows, never for one the stack hides. Client-side.
    #[hook(
        side = "client",
        contract = "cancel",
        out = "cancel",
        neoforge = "RenderTooltipEvent.GatherComponents",
        cancellable = true
    )]
    fn render_tooltip_gather_components();
}
