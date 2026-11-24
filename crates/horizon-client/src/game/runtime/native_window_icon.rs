//! `host.set_window_icon(png)`: a mod puts its brand's icon on the game window, as `Window.setIcon` does.
use super::*;

impl Game<'_> {
    pub(in crate::game) fn set_window_icon(&mut self, png: &[u8]) -> bool {
        let Some(window) = self.window.as_ref() else {
            return false;
        };
        let Some(icon) = horizon_client::window::decode_icon(png) else {
            return false;
        };
        #[cfg(target_os = "windows")]
        winit::platform::windows::WindowExtWindows::set_taskbar_icon(&**window, Some(icon.clone()));
        window.set_window_icon(Some(icon));
        true
    }
}
