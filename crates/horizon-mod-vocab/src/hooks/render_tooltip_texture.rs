//! The `render_tooltip_texture` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `RenderTooltipEvent.Texture`: the look of a tooltip box about to be drawn. Set `texture`
    /// to a tooltip style (`<namespace>:<name>`) and the box draws as vanilla draws a
    /// `tooltip_style`: the GUI sprites `<namespace>:tooltip/<name>_background` and
    /// `<namespace>:tooltip/<name>_frame` from the resource packs, each nine-sliced as its
    /// `gui.scaling` says or stretched without one. A style whose background or frame no pack
    /// has keeps the fills, where vanilla draws the missing texture. Left `None`, the box is
    /// Horizon's solid fills, in `background` and the `border_top` to `border_bottom` gradient,
    /// as ARGB. `stack` is the item the tooltip describes, `None` for a text tooltip. Asked each
    /// frame a tooltip shows, only while a mod listens. Client-side.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "RenderTooltipEvent.Texture"
    )]
    fn render_tooltip_texture();
}
