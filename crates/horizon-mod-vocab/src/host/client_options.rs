//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The client host's methods of the `client_options` area, implemented by the game and reached through [`crate::ClientHost`].
#[horizon_mod_api_macros::host_area]
pub trait ClientOptionsClientHost {
    /// Side: client.
    /// A snapshot of the current input preferences.
    #[access(ClientSetup)]
    fn input_options(&self) -> ClientInputOptions;
    /// Side: client.
    /// A snapshot of the current graphics preferences.
    #[access(ClientSetup)]
    fn graphics_options(&self) -> ClientGraphicsOptions;
    /// Side: client.
    /// A snapshot of the current GUI and accessibility preferences. Its
    /// `gui_scale` is only the option, where 0 means automatic; read
    /// [`Self::gui_scale`] for the scale the window applies.
    #[access(ClientSetup)]
    fn gui_options(&self) -> ClientGuiOptions;
    /// Side: client.
    /// The GUI scale in effect (`Window.getGuiScale`), at least 1; none on a
    /// server or when no window exists. Vanilla's `Window.calculateScale`
    /// resolves it from the GUI Scale option and the window size: the option is
    /// a maximum (0 = automatic, not a guarantee), and with the Unicode font
    /// forced an odd result is raised by one. This read returns the scale the
    /// window applies, never a separate recomputation. The framebuffer size
    /// stays `native_physical_window_size()`; divide it by this scale, rounding
    /// up, for the GUI-scaled width and height
    /// (`Window.getGuiScaledWidth/Height`).
    #[access(ClientSetup)]
    fn gui_scale(&self) -> Option<u32>;
    /// Side: client.
    /// A snapshot of the current chat preferences.
    #[access(ClientSetup)]
    fn chat_options(&self) -> ClientChatOptions;
    /// Side: client.
    /// A snapshot of the current audio preferences.
    #[access(ClientSetup)]
    fn audio_options(&self) -> ClientAudioOptions;
    /// Side: client.
    /// The live glyph for a vanilla gameplay mapping id such as `key.jump` or `key.attack`.
    /// Uses the last input method and that device's current binding. Returns `None` for unknown or
    /// unbound actions, including actions without a button binding in controller mode.
    #[access(ClientSetup)]
    #[default_on_unavailable]
    fn input_glyph(&self, action: &str) -> Option<InputGlyph>;
    /// Side: client.
    /// The live keyboard/mouse glyph for `mod_id`'s registered mapping `id`, modifiers included.
    /// Returns `None` for unknown or unbound mappings and in controller mode (registered key
    /// mappings have no controller binding).
    #[access(ClientSetup)]
    #[default_on_unavailable]
    fn mod_input_glyph(&self, mod_id: &str, id: &str) -> Option<InputGlyph>;
    /// Side: client.
    /// The glyph for an explicit vanilla physical key or mouse name (`key.keyboard.r`,
    /// `key.mouse.left`, etc.), independent of which device was used last. Unknown names are `None`.
    #[access(ClientSetup)]
    #[default_on_unavailable]
    fn key_input_glyph(&self, key: &str) -> Option<InputGlyph>;
    /// Side: client.
    /// The glyph for a physical button on this player's controller, using the selected artwork
    /// family. Does not map Nintendo letters to gameplay roles. Returns `None` if no controller
    /// is available or controllers are disabled.
    #[access(ClientSetup)]
    #[default_on_unavailable]
    fn controller_input_glyph(&self, button: ControllerButton) -> Option<InputGlyph>;
    /// Side: client.
    /// Whether fullbright is on as the player sees it: Horizon's extra that draws the
    /// world at full light, unless the server this window plays on holds it off
    /// (`settings.fullbright_held`: another's server, #3823/#14335). In single player, as a
    /// LAN host and in Replay Studio it is the extra's own switch.
    #[access(ClientSetup)]
    fn fullbright(&self) -> bool;
    /// Side: client.
    /// Turns fullbright on or off, saved with the rest of the settings as the Video
    /// settings' switch does; the next frame draws it. `false`, changing nothing, where
    /// [`Self::fullbright`] is held off by the server.
    #[access(ClientSetup)]
    fn set_fullbright(&mut self, on: bool) -> bool;
}
