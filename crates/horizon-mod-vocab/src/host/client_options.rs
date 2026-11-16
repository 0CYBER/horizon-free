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
}
