//! Decorations and hover tooltips for saved rows on the Multiplayer screen.

pub trait Hooks {
    /// Draw over one visible saved server row, after the list draws its entries and before its
    /// tooltips. Does not run for hidden or offscreen servers, the LAN scanner or its games, or
    /// other pages such as Add and Edit Server.
    ///
    /// `name` and `address` are the local saved entry's strings. Treat the address as private
    /// player data; don't log or transmit it. `rect` is the full scrolled row in GUI pixels,
    /// `[x, y, width, height]`, and `visible_rect` is its intersection with the list viewport.
    /// Draw using `rect` as the origin; drawing is clipped to `visible_rect`.
    ///
    /// `mouse` is the pointer in the same GUI coordinates when inside `visible_rect`, otherwise
    /// `None`. If hovered, append lines to `tooltip` to show them at the pointer after the screen
    /// contents, uncut by the row. Native ping and player-list tooltip lines still appear first.
    /// Lines supplied when not hovered are ignored.
    #[hook(side = "client", contract = "collect", out = "tooltip")]
    fn server_list_row_decorations(
        name: &str,
        address: &str,
        rect: [f32; 4],
        visible_rect: [f32; 4],
        mouse: Option<[f32; 2]>,
        draw: &mut dyn DrawContext,
        tooltip: &mut Vec<Text>,
    );
}
