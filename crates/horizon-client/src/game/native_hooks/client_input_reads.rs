use super::*;
use crate::game::api_values::TryIntoApi;

/// Helpers for native mods, kept as stable as the hook points.
// Native mods use these; nothing in Horizon does.
#[allow(dead_code)]
impl Game<'_> {
    /// Playing, with nothing over the world: keys and zoom apply.
    pub(in crate::game) fn playing(&self) -> bool {
        self.screen == Screen::Playing
            && self.title.is_none()
            && self.sign_edit.is_none()
            && self.book.is_none()
    }
    /// Milliseconds on the game's own clock (`Util.getMillis`), counting from the game's start:
    /// the clock the HUD's scrolling labels and the screens' text fields read too.
    pub(in crate::game) fn millis(&self) -> u64 {
        horizon_client::hud_overlays::now_ms().max(0) as u64
    }
    /// The current window modifier keys in the same GLFW encoding as input hooks.
    /// Cleared when the window loses focus; available even outside a world.
    pub(in crate::game) fn modifiers(&self) -> ClientModifiers {
        ClientModifiers::new(crate::game::glfw_modifiers(self.modifiers))
    }
    /// A vanilla mapping (`key.forward`, `key.jump`, `key.attack`…): whether
    /// it is held (`KeyMapping.isDown`), and its key's name as Controls shows
    /// it. None for a mapping it doesn't answer for.
    pub(in crate::game) fn mapping(
        &self,
        name: &str,
    ) -> Result<Option<mod_api::KeyMapping>, mod_api::TextFormError> {
        let Some(&(_, key)) = MAPPINGS.iter().find(|m| m.0 == name) else {
            return Ok(None);
        };
        let down = match key {
            Some(k) => self.movement_key_held(k),
            // The button, or the mapping's own held state (a toggle, a pad, `set_mapping_down`).
            None => {
                let use_key = usize::from(name == "key.use");
                self.mouse_held()[use_key] || [self.left, self.right][use_key]
            }
        };
        Ok(Some(mod_api::KeyMapping {
            down,
            label: keybinds::display_name(self.settings.keys.get(name)).try_into_api()?,
        }))
    }
}
