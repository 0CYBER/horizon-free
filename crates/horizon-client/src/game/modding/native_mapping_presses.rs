//! A mod presses the vanilla gameplay mappings (`ClientHost::set_mapping_down`, `click_mapping`):
//! it sets the held state a key or button gives a mapping (`KeyMapping.setDown`) or adds the click
//! a press adds (`KeyMapping.click`), and nothing more. The client tick's key step acts on them
//! with all its checks, as it does for the player's own keys, so the server hears the packets of a
//! normal client at the vanilla moments.
use super::Game;
use horizon_client::held::Source;
use horizon_client::keybinds;
use horizon_client::keymapping::{SNEAK, SPRINT};
use mod_api::MappingError;
use winit::{event::MouseButton, keyboard::KeyCode};

/// The default key the game's key handling hears for the vanilla mapping `key` (`Ok` a key, `Err`
/// a mouse button), when it is one a player plays with: a mapping of the movement, gameplay or
/// inventory category.
fn gameplay_mapping(key: &str) -> Result<Result<KeyCode, MouseButton>, MappingError> {
    let unknown = || MappingError::Unknown { key: key.into() };
    let &(_, default, category) = keybinds::MAPPINGS
        .iter()
        .find(|m| m.0 == key)
        .ok_or_else(unknown)?;
    if !matches!(category, "movement" | "gameplay" | "inventory") {
        return Err(MappingError::NotGameplay { key: key.into() });
    }
    keybinds::key_code(default)
        .map(Ok)
        .or_else(|| keybinds::mouse_button(default).map(Err))
        .ok_or_else(unknown)
}

/// The toggle slot of sneak's and sprint's default keys (`ToggleKeys`).
fn toggle_slot(key: KeyCode) -> Option<usize> {
    match key {
        KeyCode::ShiftLeft => Some(SNEAK),
        KeyCode::ControlLeft => Some(SPRINT),
        _ => None,
    }
}

impl Game<'_> {
    /// `KeyMapping.setDown` for the mapping on `button`: attack and use, toggled by their option;
    /// nothing reads the pick mapping's held state.
    pub(super) fn mouse_mapping_set_down(&mut self, button: MouseButton, down: bool) {
        let (slot, option) = match button {
            MouseButton::Left => (2, self.settings.toggle_attack),
            MouseButton::Right => (3, self.settings.toggle_use),
            _ => return,
        };
        self.toggles.0[slot].set_down(down, option);
        self.left = self.toggles.is_down(2);
        self.right = self.toggles.is_down(3);
    }
    /// `KeyMapping.click` for the mapping on `button`: on a server, the queued click
    /// `handleKeybinds` takes on the next tick (`interaction_input`); with no server, the attack
    /// and use clicks `tick_local_hand` takes, and pick block at once.
    pub(super) fn mouse_mapping_click(&mut self, button: MouseButton) {
        use horizon_client::interaction_input::Button;
        if let Some(slot) = match button {
            MouseButton::Left => Some(2),
            MouseButton::Right => Some(3),
            _ => None,
        } {
            self.toggles.0[slot].click();
        }
        if let Some(remote) = self.remote.as_mut() {
            let input = match button {
                MouseButton::Left => Button::Attack,
                MouseButton::Right => Button::Use,
                MouseButton::Middle => Button::Pick,
                _ => return,
            };
            remote.input.press(input);
            return;
        }
        // Mod callbacks may enqueue repeatedly before a local tick: never wrap the counters.
        match button {
            // KeyMapping.click: Minecraft.tick starts the attack.
            MouseButton::Left => self.attack_clicks = self.attack_clicks.saturating_add(1),
            // handleKeybinds consumes use clicks without acting while an
            // item is in use. Minecraft.tick runs them after the attack
            // clicks, so a place and a break in one tick replace the block.
            MouseButton::Right => {
                if self.using.is_none() {
                    self.use_clicks = self.use_clicks.saturating_add(1);
                }
            }
            MouseButton::Middle => {
                // handleKeybinds: the number keys select before key.pickItem.
                self.select_queued_hotbar();
                // Minecraft.pickBlockOrEntity -> the server's handlePickItemFromBlock:
                // the block's clone item, with its block entity's data on Ctrl in
                // creative, picked from the inventory (`pick_item`).
                if let Some(entity) = self.pick_entity_hit() {
                    // handlePickItemFromEntity: the entity's pick result.
                    if let Some(stack) = entity.and_then(|id| self.entity_pick_result(id)) {
                        let infinite = self.creative;
                        horizon_client::pick_item::pick(
                            &mut self.inventory[..36],
                            &mut self.selected,
                            &stack,
                            infinite,
                        );
                    }
                } else if let Some(hit) = self.pick() {
                    let include_data = self.creative && self.has_control_down();
                    if let Some(stack) = self.picked_stack(hit.pos, include_data) {
                        let infinite = self.creative;
                        horizon_client::pick_item::pick(
                            &mut self.inventory[..36],
                            &mut self.selected,
                            &stack,
                            infinite,
                        );
                    }
                }
            }
            _ => {}
        }
    }
    /// `KeyMapping.setDown` for the vanilla gameplay mapping `key`, from a mod: attack and use as
    /// their mouse button sets them, sneak and sprint toggled by their option, the other keys held
    /// as their key holds them. With a screen or overlay open a press does nothing: the mappings
    /// hear none behind it, and its opening let go of all of them (`KeyMapping.releaseAll`).
    pub(in crate::game) fn set_mapping_down(
        &mut self,
        key: &str,
        down: bool,
    ) -> Result<(), MappingError> {
        let default = self.pressable_mapping(key)?;
        if down && self.mod_mappings_screen_open() {
            return Ok(());
        }
        match default {
            Err(button) => self.mouse_mapping_set_down(button, down),
            Ok(key) => match toggle_slot(key) {
                Some(slot) => {
                    let option = [self.settings.toggle_crouch, self.settings.toggle_sprint][slot];
                    self.toggles.0[slot].set_down(down, option);
                    self.sync_toggle(slot);
                    // The mod holds it, whatever the keyboard's modifier says (`heal_hold_latches`).
                    if self.toggles.is_down(slot) {
                        self.keys.press(key, Source::Mod);
                    }
                }
                None if down => {
                    self.keys.press(key, Source::Mod);
                }
                None => {
                    self.keys.release(&key, Source::Mod);
                }
            },
        }
        Ok(())
    }
    /// `KeyMapping.click` for the vanilla gameplay mapping `key`, from a mod: the attack, use and
    /// pick clicks the next tick's `handleKeybinds` takes, and a key's press as `key_pressed`
    /// hears it (the hotbar, drop, inventory and hand swap). Behind a screen or overlay the click
    /// is dropped, as its opening drops every click.
    pub(in crate::game) fn click_mapping(&mut self, key: &str) -> Result<(), MappingError> {
        let default = self.pressable_mapping(key)?;
        if self.mod_mappings_screen_open() {
            return Ok(());
        }
        match default {
            Err(button) => self.mouse_mapping_click(button),
            // Nothing takes sneak's or sprint's clicks.
            Ok(code) => match toggle_slot(code) {
                Some(slot) => self.toggles.0[slot].click(),
                None => {
                    if let Err(error) = self.key_pressed(code, code) {
                        log::warn!("click_mapping {key}: {error:#}");
                    }
                }
            },
        }
        Ok(())
    }
    /// The default key of a gameplay mapping a mod may press now: refused by name, or with no
    /// player in a world.
    fn pressable_mapping(&self, key: &str) -> Result<Result<KeyCode, MouseButton>, MappingError> {
        let default = gameplay_mapping(key)?;
        if self.player.is_none() {
            return Err(MappingError::NoWorld);
        }
        Ok(default)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The names a mod presses, and the ones refused by name.
    #[test]
    fn gameplay_mappings_resolve_and_the_others_are_refused_by_name() {
        assert_eq!(gameplay_mapping("key.attack"), Ok(Err(MouseButton::Left)));
        assert_eq!(gameplay_mapping("key.use"), Ok(Err(MouseButton::Right)));
        assert_eq!(
            gameplay_mapping("key.pickItem"),
            Ok(Err(MouseButton::Middle))
        );
        assert_eq!(gameplay_mapping("key.sprint"), Ok(Ok(KeyCode::ControlLeft)));
        assert_eq!(gameplay_mapping("key.forward"), Ok(Ok(KeyCode::KeyW)));
        assert_eq!(gameplay_mapping("key.hotbar.3"), Ok(Ok(KeyCode::Digit3)));
        assert_eq!(gameplay_mapping("key.inventory"), Ok(Ok(KeyCode::KeyE)));
        assert_eq!(gameplay_mapping("key.drop"), Ok(Ok(KeyCode::KeyQ)));
        assert_eq!(gameplay_mapping("key.swapOffhand"), Ok(Ok(KeyCode::KeyF)));
        for key in [
            "key.screenshot",
            "key.fullscreen",
            "key.chat",
            "key.spectatorHotbar",
        ] {
            assert_eq!(
                gameplay_mapping(key),
                Err(MappingError::NotGameplay { key: key.into() })
            );
        }
        for key in ["nope", "key.emote", ""] {
            assert_eq!(
                gameplay_mapping(key),
                Err(MappingError::Unknown { key: key.into() })
            );
        }
    }
}
