use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
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
    pub(in crate::game) fn native_render_tooltip_gather_components(
        &self,
        stack: &ItemStack,
        screen_width: i32,
        screen_height: i32,
        elements: &mut Vec<TooltipElement>,
        max_width: &mut i32,
        cancel: &mut bool,
    ) {
        self.fire_render_tooltip_gather_components(
            stack,
            screen_width,
            screen_height,
            elements,
            max_width,
            cancel,
        );
    }
}
