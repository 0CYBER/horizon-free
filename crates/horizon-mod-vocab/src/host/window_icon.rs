//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]

/// The client host's methods of the `window_icon` area, implemented by the game and reached through [`crate::ClientHost`].
#[horizon_mod_api_macros::host_area]
pub trait WindowIconClientHost {
    /// Side: client.
    /// Puts `png` (an RGBA or palette PNG, read from the mod's own resources with `resource`) on the
    /// game window's title bar and, on Windows, the taskbar, as `Window.setIcon` does for a modpack's
    /// brand. False, with the icon unchanged, when there is no window or the PNG doesn't decode. macOS
    /// takes the icon from the app bundle and Wayland from the `.desktop` entry: there the call answers
    /// true and nothing shows, as vanilla's.
    #[access(ClientSetup)]
    fn set_window_icon(&mut self, png: &[u8]) -> bool;
}
