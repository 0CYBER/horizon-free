//! The down/click state of `KeyMapping` and `ToggleKeyMapping` (sneak,
//! sprint, attack and use can be toggled instead of held).

/// One mapping's runtime state. `toggle` is `Some(restore)` for a
/// `ToggleKeyMapping` (with its `shouldRestore` flag), `None` for a plain one.
#[derive(Clone, Debug)]
pub struct KeyState {
    pub is_down: bool,
    click_count: i32,
    released_by_screen_when_down: bool,
    toggle: Option<bool>,
    keyboard: bool,
}

impl KeyState {
    pub fn plain() -> Self {
        Self {
            is_down: false,
            click_count: 0,
            released_by_screen_when_down: false,
            toggle: None,
            keyboard: true,
        }
    }
    /// A `ToggleKeyMapping`; `keyboard` is whether the bound key is a keyboard key.
    pub fn toggle(should_restore: bool, keyboard: bool) -> Self {
        Self {
            toggle: Some(should_restore),
            keyboard,
            ..Self::plain()
        }
    }
    /// `KeyMapping.click`: one more pending click.
    pub fn click(&mut self) {
        self.click_count += 1;
    }
    /// `consumeClick`.
    pub fn consume_click(&mut self) -> bool {
        if self.click_count == 0 {
            false
        } else {
            self.click_count -= 1;
            true
        }
    }
    /// `setDown`; `needs_toggle` is the mapping's option value.
    pub fn set_down(&mut self, down: bool, needs_toggle: bool) {
        if self.toggle.is_some() && needs_toggle {
            if down {
                self.is_down = !self.is_down;
            }
        } else {
            self.is_down = down;
        }
    }
    /// `shouldSetOnIngameFocus` for a keyboard-bound, bound key.
    pub fn should_set_on_ingame_focus(&self, needs_toggle: bool) -> bool {
        self.keyboard && (self.toggle.is_none() || !needs_toggle)
    }
    /// `release` (a screen opened): drops pending clicks and the held state.
    pub fn release(&mut self, needs_toggle: bool) {
        if self.toggle.is_some() {
            // ToggleKeyMapping.release does not clear pending clicks.
            if needs_toggle && self.is_down || self.released_by_screen_when_down {
                self.released_by_screen_when_down = true;
            }
            self.reset();
        } else {
            self.click_count = 0;
            self.set_down(false, needs_toggle);
        }
    }
    /// `ToggleKeyMapping.reset`.
    pub fn reset(&mut self) {
        self.is_down = false;
    }
    /// `shouldRestoreStateOnScreenClosed` (clears the remembered release).
    pub fn should_restore_state_on_screen_closed(&mut self, needs_toggle: bool) -> bool {
        let Some(restore) = self.toggle else {
            return false;
        };
        let result = restore && needs_toggle && self.keyboard && self.released_by_screen_when_down;
        self.released_by_screen_when_down = false;
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn toggle_flips_on_press_and_ignores_release() {
        let mut k = KeyState::toggle(true, true);
        k.set_down(true, true);
        assert!(k.is_down);
        k.set_down(false, true);
        assert!(k.is_down);
        k.set_down(true, true);
        assert!(!k.is_down);
    }
    #[test]
    fn screen_release_restores_toggled_state() {
        let mut k = KeyState::toggle(true, true);
        k.set_down(true, true);
        k.release(true);
        assert!(!k.is_down);
        assert!(k.should_restore_state_on_screen_closed(true));
        assert!(!k.should_restore_state_on_screen_closed(true));
    }
}
