use super::*;

/// Helpers for native mods, kept as stable as the hook points.
// Native mods use these; nothing in Horizon does.
#[allow(dead_code)]
impl Game<'_> {
    /// A snapshot of the current input preferences.
    pub(in crate::game) fn input_options(&self) -> ClientInputOptions {
        let settings = &self.settings;
        ClientInputOptions {
            sensitivity: settings.sensitivity,
            invert_x: settings.invert_x,
            invert_y: settings.invert_y,
            mouse_wheel_sensitivity: settings.mouse_wheel_sensitivity,
            discrete_scroll: settings.discrete_scroll,
            toggle_crouch: settings.toggle_crouch,
            toggle_sprint: settings.toggle_sprint,
            auto_jump: settings.auto_jump,
        }
    }
}
