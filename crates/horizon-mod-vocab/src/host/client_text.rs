//! Client text resolution and styled measurements, outside drawing.
#![allow(unused_variables)]
use crate::*;

/// Read-only component resolution and access to the client tooltip font.
#[horizon_mod_api_macros::host_area]
pub trait ClientTextClientHost {
    /// Side: client.
    /// `Component.getString()`: resolve a component to its plain characters,
    /// including current-language translations, arguments, key names and siblings.
    /// Styles do not contribute characters; literal legacy formatting codes remain
    /// in the string, as in vanilla. Available outside drawing, including client
    /// setup, on native and wasm. Invalid components return their boundary error.
    /// Action keybind IDs such as `key.jump` resolve through the client's current
    /// key binding to their assigned physical key, matching vanilla KeybindContents.
    #[access(ClientSetup)]
    fn text_string(&self, text: &Text) -> Result<String, TextFormError>;

    /// Side: client.
    /// `Component.getVisualOrderText()`: current-language, styled characters in
    /// visual order, including bidi shaping and mirroring. Available outside
    /// drawing on native and wasm, including client setup. Invalid components
    /// return the same boundary errors as `text_string`. Sink indices are local
    /// UTF-16 offsets, not offsets into the input component tree.
    /// Action keybinds resolve to their assigned keys as in `text_string`.
    /// No NeoForge hook counterpart is documented.
    #[access(ClientSetup)]
    fn text_visual_order(&self, text: &Text) -> Result<Vec<VisualTextCharacter>, TextFormError>;

    /// Side: client.
    /// `Font.width(FormattedText)`: the width of a component in unscaled GUI
    /// pixels, using the current language, styles and resource-pack fonts.
    /// Available outside a draw, including client setup and tooltip-component
    /// sizing. Like `ClientTooltipComponent.getWidth(Font)`, this measures with
    /// the rendering client's active font, not a font chosen by the mod.
    /// Build owned components with `Text`, or validate a flat `TextForm` with
    /// `Text::try_from(form)`. Invalid component input returns its boundary error.
    #[access(ClientSetup)]
    fn text_width(&self, text: &Text) -> Result<i32, TextFormError>;

    /// Side: client.
    /// `Font.split(FormattedText, width)`: wrap a component at `width` unscaled
    /// GUI pixels. Lines retain styles, resolve the current language and use
    /// the same active font as tooltip layout. Newlines force breaks; a width
    /// below one follows vanilla's one-pixel minimum, without rejecting input.
    /// Available outside drawing on native and wasm alike.
    #[access(ClientSetup)]
    fn text_split(&self, text: &Text, width: i32) -> Result<Vec<Text>, TextFormError>;
}
