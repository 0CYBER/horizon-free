//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The server host's methods of the `displays` area, implemented by the game and reached through [`ServerHost`].
#[horizon_mod_api_macros::host_area]
pub trait DisplaysServerHost {
    /// Side: server.
    /// Narrows an entity to a display, or None for another kind, a stale
    /// handle or a client.
    #[access(WorldOnly)]
    fn display(&self, entity: EntityRef) -> Option<DisplayRef>;
}
/// The handles' reads of the `displays` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[horizon_mod_api_macros::host_area]
pub trait DisplaysBackend {
    /// Side: server.
    /// Which `Display` subclass the entity is.
    #[access(WorldOnly)]
    fn display_kind(&self, display: DisplayRef) -> Option<DisplayKind>;
    /// Side: server.
    /// `Display.createTransformation`: the transformation as shown.
    #[access(WorldOnly)]
    fn display_transformation(&self, display: DisplayRef) -> Option<Transformation>;
    /// Side: server.
    /// `Display.getBillboardConstraints`.
    #[access(WorldOnly)]
    fn display_billboard(&self, display: DisplayRef) -> Option<Billboard>;
    /// Side: server.
    /// `Display.getBrightnessOverride`: block and sky light, 0 to 15 each, or None for no override.
    #[access(WorldOnly)]
    fn display_brightness(&self, display: DisplayRef) -> Option<(u8, u8)>;
    /// Side: server.
    /// `Display.getViewRange`.
    #[access(WorldOnly)]
    fn display_view_range(&self, display: DisplayRef) -> Option<f32>;
    /// Side: server.
    /// `Display.getShadowRadius`.
    #[access(WorldOnly)]
    fn display_shadow_radius(&self, display: DisplayRef) -> Option<f32>;
    /// Side: server.
    /// `Display.getShadowStrength`.
    #[access(WorldOnly)]
    fn display_shadow_strength(&self, display: DisplayRef) -> Option<f32>;
    /// Side: server.
    /// `Display.getWidth`: the culling box's width, 0 for none.
    #[access(WorldOnly)]
    fn display_width(&self, display: DisplayRef) -> Option<f32>;
    /// Side: server.
    /// `Display.getHeight`: the culling box's height, 0 for none.
    #[access(WorldOnly)]
    fn display_height(&self, display: DisplayRef) -> Option<f32>;
    /// Side: server.
    /// `Display.getGlowColorOverride`: an ARGB color, or -1 for none.
    #[access(WorldOnly)]
    fn display_glow_color_override(&self, display: DisplayRef) -> Option<i32>;
    /// Side: server.
    /// `Display.getTransformationInterpolationDuration`, in ticks.
    #[access(WorldOnly)]
    fn display_interpolation_duration(&self, display: DisplayRef) -> Option<i32>;
    /// Side: server.
    /// `Display.getTransformationInterpolationDelay`: ticks before an
    /// interpolation starts.
    #[access(WorldOnly)]
    fn display_start_interpolation(&self, display: DisplayRef) -> Option<i32>;
    /// Side: server.
    /// `Display.getPosRotInterpolationDuration`: ticks a move eases over, 0 to 59.
    #[access(WorldOnly)]
    fn display_teleport_duration(&self, display: DisplayRef) -> Option<i32>;
    /// Side: server.
    /// `TextDisplay.getText`; None for another kind of display.
    #[access(WorldOnly)]
    /// Refuses text deeper than 512 component nodes before applying any changes.
    fn display_text(&self, display: DisplayRef) -> Result<Option<Text>, TextFormError>;
    /// Side: server.
    /// `TextDisplay.getLineWidth`.
    #[access(WorldOnly)]
    fn display_line_width(&self, display: DisplayRef) -> Option<i32>;
    /// Side: server.
    /// `TextDisplay.getBackgroundColor`: ARGB.
    #[access(WorldOnly)]
    fn display_background(&self, display: DisplayRef) -> Option<i32>;
    /// Side: server.
    /// `TextDisplay.getTextOpacity`: -1 is opaque, else 0 to 127 then -128 to -2.
    #[access(WorldOnly)]
    fn display_text_opacity(&self, display: DisplayRef) -> Option<i8>;
    /// Side: server.
    /// `TextDisplay.FLAG_SHADOW`.
    #[access(WorldOnly)]
    fn display_text_shadow(&self, display: DisplayRef) -> Option<bool>;
    /// Side: server.
    /// `TextDisplay.FLAG_SEE_THROUGH`.
    #[access(WorldOnly)]
    fn display_see_through(&self, display: DisplayRef) -> Option<bool>;
    /// Side: server.
    /// `TextDisplay.FLAG_USE_DEFAULT_BACKGROUND`.
    #[access(WorldOnly)]
    fn display_default_background(&self, display: DisplayRef) -> Option<bool>;
    /// Side: server.
    /// `TextDisplay.getAlign`, from the left and right flags.
    #[access(WorldOnly)]
    fn display_alignment(&self, display: DisplayRef) -> Option<TextAlignment>;
    /// Side: server.
    /// `BlockDisplay.getBlockState`; None for another kind of display.
    #[access(WorldOnly)]
    fn display_block_state(&self, display: DisplayRef) -> Option<BlockState>;
    /// Side: server.
    /// `ItemDisplay.getItemStack`; None for another kind of display or an empty stack.
    #[access(WorldOnly)]
    fn display_item(&self, display: DisplayRef) -> Option<ItemStack>;
    /// Side: server.
    /// `ItemDisplay.getItemTransform`: how the item is displayed.
    #[access(WorldOnly)]
    fn display_item_context(&self, display: DisplayRef) -> Option<ItemDisplayContext>;
}
/// The handles' writes of the `displays` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[horizon_mod_api_macros::host_area]
pub trait DisplaysBackendMut {
    /// Side: server.
    /// Sets the transformation. False for a value that isn't finite or a
    /// zero quaternion. Clients ease to it when the start delay is set
    /// (`set_start_interpolation`).
    #[access(WorldOnly)]
    fn display_set_transformation(
        &mut self,
        display: DisplayRef,
        transformation: Transformation,
    ) -> bool;
    /// Side: server.
    /// `Display.setBillboardConstraints`.
    #[access(WorldOnly)]
    fn display_set_billboard(&mut self, display: DisplayRef, billboard: Billboard) -> bool;
    /// Side: server.
    /// `Display.setBrightnessOverride`; None clears the override. False for a
    /// light level above 15.
    #[access(WorldOnly)]
    fn display_set_brightness(&mut self, display: DisplayRef, brightness: Option<(u8, u8)>)
    -> bool;
    /// Side: server.
    /// `Display.setViewRange`. False for a value that isn't finite.
    #[access(WorldOnly)]
    fn display_set_view_range(&mut self, display: DisplayRef, range: f32) -> bool;
    /// Side: server.
    /// `Display.setShadowRadius`. False for a value that isn't finite.
    #[access(WorldOnly)]
    fn display_set_shadow_radius(&mut self, display: DisplayRef, radius: f32) -> bool;
    /// Side: server.
    /// `Display.setShadowStrength`. False for a value that isn't finite.
    #[access(WorldOnly)]
    fn display_set_shadow_strength(&mut self, display: DisplayRef, strength: f32) -> bool;
    /// Side: server.
    /// `Display.setWidth`. False for a value that isn't finite.
    #[access(WorldOnly)]
    fn display_set_width(&mut self, display: DisplayRef, width: f32) -> bool;
    /// Side: server.
    /// `Display.setHeight`. False for a value that isn't finite.
    #[access(WorldOnly)]
    fn display_set_height(&mut self, display: DisplayRef, height: f32) -> bool;
    /// Side: server.
    /// `Display.setGlowColorOverride`.
    #[access(WorldOnly)]
    fn display_set_glow_color_override(&mut self, display: DisplayRef, color: i32) -> bool;
    /// Side: server.
    /// `Display.setTransformationInterpolationDuration`.
    #[access(WorldOnly)]
    fn display_set_interpolation_duration(&mut self, display: DisplayRef, ticks: i32) -> bool;
    /// Side: server.
    /// `Display.setTransformationInterpolationDelay`. Like vanilla, every
    /// call is sent to the clients that track the display, even with the
    /// delay it already has, and each client restarts its interpolation.
    #[access(WorldOnly)]
    fn display_set_start_interpolation(&mut self, display: DisplayRef, ticks: i32) -> bool;
    /// Side: server.
    /// `Display.setPosRotInterpolationDuration`, kept from 0 to 59 as
    /// vanilla's load keeps it.
    #[access(WorldOnly)]
    fn display_set_teleport_duration(&mut self, display: DisplayRef, ticks: i32) -> bool;
    /// Side: server.
    /// `TextDisplay.setText`; false for another kind of display.
    #[access(WorldOnly)]
    /// Refuses text deeper than 512 component nodes before applying any changes.
    fn display_set_text(&mut self, display: DisplayRef, text: Text) -> Result<bool, TextFormError>;
    /// Side: server.
    /// `TextDisplay.setLineWidth`; false for another kind of display.
    #[access(WorldOnly)]
    fn display_set_line_width(&mut self, display: DisplayRef, width: i32) -> bool;
    /// Side: server.
    /// `TextDisplay.setBackgroundColor`; false for another kind of display.
    #[access(WorldOnly)]
    fn display_set_background(&mut self, display: DisplayRef, argb: i32) -> bool;
    /// Side: server.
    /// `TextDisplay.setTextOpacity`; false for another kind of display.
    #[access(WorldOnly)]
    fn display_set_text_opacity(&mut self, display: DisplayRef, opacity: i8) -> bool;
    /// Side: server.
    /// Sets the shadow flag; false for another kind of display.
    #[access(WorldOnly)]
    fn display_set_text_shadow(&mut self, display: DisplayRef, on: bool) -> bool;
    /// Side: server.
    /// Sets the see-through flag; false for another kind of display.
    #[access(WorldOnly)]
    fn display_set_see_through(&mut self, display: DisplayRef, on: bool) -> bool;
    /// Side: server.
    /// Sets the default-background flag; false for another kind of display.
    #[access(WorldOnly)]
    fn display_set_default_background(&mut self, display: DisplayRef, on: bool) -> bool;
    /// Side: server.
    /// Sets the alignment; false for another kind of display.
    #[access(WorldOnly)]
    fn display_set_alignment(&mut self, display: DisplayRef, alignment: TextAlignment) -> bool;
    /// Side: server.
    /// `BlockDisplay.setBlockState`; false for another kind of display or an unknown state.
    #[access(WorldOnly)]
    fn display_set_block_state(&mut self, display: DisplayRef, state: BlockState) -> bool;
    /// Side: server.
    /// `ItemDisplay.setItemStack`; None empties it. False for another kind of display.
    #[access(WorldOnly)]
    fn display_set_item(
        &mut self,
        display: DisplayRef,
        stack: Option<ItemStack>,
    ) -> Result<bool, StackWriteError>;
    /// Side: server.
    /// `ItemDisplay.setItemTransform`; false for another kind of display.
    #[access(WorldOnly)]
    fn display_set_item_context(
        &mut self,
        display: DisplayRef,
        context: ItemDisplayContext,
    ) -> bool;
}
