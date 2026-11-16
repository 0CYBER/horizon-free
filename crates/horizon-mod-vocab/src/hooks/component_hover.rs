//! The `component_hover` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `GuiGraphicsExtractor.componentHoverEffect`: the hover of the text component under the
    /// cursor, about to be drawn, in the chat, a book or a dialog. `hover` is the component's
    /// hover action: its text, an item or an entity. `x` and `y` are the cursor in GUI pixels.
    /// Set `cancel` and the game draws nothing for it: no text tooltip, no item tooltip and no
    /// entity lines. A mod can draw its own with `draw`. Left alone, a text hover is wrapped at
    /// half the screen's width (at least 200) as vanilla wraps it, an item hover draws its stack's
    /// tooltip (which asks `item_tooltip` and the `render_tooltip_*` hooks as any item tooltip
    /// does), and an entity hover shows its name, type and UUID while advanced tooltips are on.
    /// Asked each frame a hovered component shows, only while a mod listens. Client-side.
    #[hook(
        side = "client",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (vanilla GuiGraphicsExtractor.componentHoverEffect; NeoForge adds no event)",
        cancellable = true
    )]
    fn component_hover(
        draw: &mut dyn DrawContext,
        hover: &TextHover,
        x: i32,
        y: i32,
        cancel: &mut bool,
    );
}
