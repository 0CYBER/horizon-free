//! The screen a window is on.

/// The screen `window` is on, else the primary one, else the only one: right after the window
/// is made, Wayland doesn't say yet which screen it is on.
pub fn of(window: &winit::window::Window) -> Option<winit::monitor::MonitorHandle> {
    let only = || {
        let mut all = window.available_monitors();
        all.next().filter(|_| all.next().is_none())
    };
    window
        .current_monitor()
        .or_else(|| window.primary_monitor())
        .or_else(only)
}
