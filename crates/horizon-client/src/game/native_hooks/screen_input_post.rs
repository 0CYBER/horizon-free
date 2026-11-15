use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// A mouse press after the open screen's arms ran (`ScreenEvent.MouseButtonPressed.Post`).
    pub(in crate::game) fn native_screen_mouse_button_pressed_post(
        &self,
        screen: &ScreenView<'_>,
        button: i32,
        modifiers: u32,
    ) {
        self.fire_screen_mouse_button_pressed_post(screen, button, modifiers);
    }
    /// A mouse release after the open screen's arms ran (`ScreenEvent.MouseButtonReleased.Post`).
    pub(in crate::game) fn native_screen_mouse_button_released_post(
        &self,
        screen: &ScreenView<'_>,
        button: i32,
        modifiers: u32,
    ) {
        self.fire_screen_mouse_button_released_post(screen, button, modifiers);
    }
    /// A drag the open screen's handlers have seen (`ScreenEvent.MouseDragged.Post`).
    pub(in crate::game) fn native_screen_mouse_dragged_post(
        &self,
        screen: &ScreenView<'_>,
        button: i32,
        delta: [f32; 2],
        modifiers: u32,
    ) {
        self.fire_screen_mouse_dragged_post(screen, button, delta, modifiers);
    }
    /// A wheel turn the open screen has scrolled by (`ScreenEvent.MouseScrolled.Post`).
    pub(in crate::game) fn native_screen_mouse_scrolled_post(
        &self,
        screen: &ScreenView<'_>,
        scroll: [f64; 2],
        modifiers: u32,
    ) {
        self.fire_screen_mouse_scrolled_post(screen, scroll, modifiers);
    }
    /// A key press no screen took (`ScreenEvent.KeyPressed.Post`).
    pub(in crate::game) fn native_screen_key_pressed_post(
        &self,
        screen: &ScreenView<'_>,
        key: &str,
        repeat: bool,
        modifiers: u32,
        canceled: &mut bool,
    ) {
        self.fire_screen_key_pressed_post(screen, key, repeat, modifiers, canceled);
    }
    /// A key release no screen took (`ScreenEvent.KeyReleased.Post`).
    pub(in crate::game) fn native_screen_key_released_post(
        &self,
        screen: &ScreenView<'_>,
        key: &str,
        modifiers: u32,
        canceled: &mut bool,
    ) {
        self.fire_screen_key_released_post(screen, key, modifiers, canceled);
    }
    /// A character no text box took (`ScreenEvent.CharacterTyped.Post`).
    pub(in crate::game) fn native_screen_char_typed_post(
        &self,
        screen: &ScreenView<'_>,
        c: char,
        modifiers: u32,
        canceled: &mut bool,
    ) {
        self.fire_screen_char_typed_post(screen, c, modifiers, canceled);
    }
}
