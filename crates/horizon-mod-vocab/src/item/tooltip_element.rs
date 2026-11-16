use crate::{ItemStack, Text};

/// One element of an item's tooltip as `render_tooltip_gather_components`
/// hands it over, before any wrapping
/// (`Either<FormattedText, TooltipComponent>`). Text deeper than 512 nodes
/// refuses tooltip conversion before any changed element is drawn. Built-in bundle
/// images share this list with text and item-part images. Gather may reorder or
/// remove them; pre sees the resulting list. The built-in image starts after
/// the name, or at index zero when there is no text.
#[derive(Clone, PartialEq)]
// exhaustive: a closed set the game matches without a wildcard
#[derive(horizon_mod_api_macros::Wire)]
pub enum TooltipElement {
    /// A line (`FormattedText`) with its colour as `rgba` packs it
    /// (`u32::MAX` keeps the text's own).
    Text(Text, u32),
    /// An image component (`TooltipComponent`).
    Image(TooltipImage),
}

/// The renderer and stack supplying an image in the ordered tooltip list.
/// Both sources reserve their measured height and draw in the image pass,
/// after all text, at their position in the list.
#[derive(Clone, PartialEq)]
// exhaustive: the game dispatches each supported image source
#[derive(horizon_mod_api_macros::Wire)]
pub enum TooltipImageSource {
    /// Drawn by the stack item's `tooltip_component_draw` part.
    ItemPart(ItemStack),
    /// The game's built-in bundle contents grid.
    Bundle(ItemStack),
}

/// An image component measured in GUI pixels, in the ordered tooltip list.
#[derive(Clone, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct TooltipImage {
    /// The renderer and its stack data.
    pub source: TooltipImageSource,
    /// This component's measurement in GUI pixels. The draw callback receives
    /// the whole tooltip content box size, not this measurement.
    pub size: [f32; 2],
}

/// Where a tooltip hangs from, in GUI pixels: what `render_tooltip_pre` lets a mod move.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: the game builds it with a literal; mods change both fields
#[derive(horizon_mod_api_macros::Wire)]
pub struct TooltipPlace {
    /// The point's x, before the positioner's offset and clamp; set it to move the tooltip.
    pub x: i32,
    /// The point's y, before the positioner's offset and clamp; set it to move the tooltip.
    pub y: i32,
}
