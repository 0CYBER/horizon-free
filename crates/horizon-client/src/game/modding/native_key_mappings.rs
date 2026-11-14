//! The keys mods register (`Registrar::register_key_mapping`) as plain `KeyMapping`s: held while
//! their binding is down and counting its presses for `consume_click`. Keys reach them on one
//! route with `input_key`, from the window and the control socket alike; mouse buttons from the
//! window and the control socket's `mouse`.
use super::Game;
use super::key_codes::{gameplay_has_keyboard, glfw_modifiers};
use horizon_client::keymapping::{KeyAction, MappingContext, ModBinding};
use mod_api::KeyConflictContext;
use winit::{event::MouseButton, keyboard::KeyCode};

impl Game<'_> {
    /// Binds the mods' mappings to the player's keys (`horizon-mods.txt`) or their declared
    /// defaults: when the settings apply, at start and after Controls changes one.
    pub(super) fn bind_mod_mappings(&mut self) {
        let mappings = super::mods::collect_key_mappings(
            self.options.execution_role,
            mod_api::__private::key_mappings(),
        );
        let settings = &self.settings;
        self.native_input
            .mappings
            .bind(mappings.iter().map(|(owner, mapping)| {
                let (input, modifiers) = settings.mods.binding(
                    owner,
                    &mapping.id,
                    (&mapping.key, mapping.modifiers.bits),
                );
                ModBinding {
                    owner,
                    id: &mapping.id,
                    input,
                    modifiers,
                    context: match mapping.context {
                        KeyConflictContext::InGame => MappingContext::InGame,
                        KeyConflictContext::Gui => MappingContext::Gui,
                        _ => MappingContext::Universal,
                    },
                }
            }));
    }
    /// One key event, the window's after it handled it or the control socket's: the mods'
    /// mappings bound to it hear it (`KeyMapping.set`, `KeyMapping.click`), then `input_key`
    /// fires. `key` is vanilla's name for it; the rest is what `native_input_key` takes.
    pub(super) fn key_input(
        &mut self,
        key: Option<&'static str>,
        layout_key: Option<&str>,
        raw_code: u32,
        action: mod_api::KeyAction,
        modifiers: u32,
    ) {
        if let Some(name) = key {
            let action = match action {
                mod_api::KeyAction::Release => KeyAction::Release,
                mod_api::KeyAction::Repeat => KeyAction::Repeat,
                _ => KeyAction::Press,
            };
            self.mod_mappings_input(name, action, false);
        }
        let key = key.unwrap_or("key.keyboard.unknown");
        self.native_input_key(key, layout_key, raw_code, action, modifiers);
    }
    /// The control socket's `key KEY down|up|tap|repeat`, through `key_input` as a window key
    /// goes: a tap is a press, then a release. A script types on a QWERTY layout.
    pub(super) fn socket_key_input(&mut self, key: KeyCode, action: &str) {
        let name = horizon_client::keybinds::key_name(key);
        let layout_key = match key {
            KeyCode::Backquote
            | KeyCode::Minus
            | KeyCode::Equal
            | KeyCode::BracketLeft
            | KeyCode::BracketRight
            | KeyCode::Backslash
            | KeyCode::IntlBackslash
            | KeyCode::Semicolon
            | KeyCode::Quote
            | KeyCode::Comma
            | KeyCode::Period
            | KeyCode::Slash => None,
            _ => name,
        };
        let raw_code =
            horizon_client::keybinds::raw_key_code(winit::keyboard::PhysicalKey::Code(key));
        let actions: &[mod_api::KeyAction] = match action {
            "down" => &[mod_api::KeyAction::Press],
            "repeat" => &[mod_api::KeyAction::Repeat],
            "up" => &[mod_api::KeyAction::Release],
            _ => &[mod_api::KeyAction::Press, mod_api::KeyAction::Release],
        };
        for action in actions {
            let modifiers = glfw_modifiers(self.modifiers);
            self.key_input(name, layout_key, raw_code, *action, modifiers);
        }
    }
    /// The same as a key's for a mouse button (`MouseHandler.onButton`).
    pub(super) fn mod_mappings_button(&mut self, button: MouseButton, pressed: bool) {
        let action = if pressed {
            KeyAction::Press
        } else {
            KeyAction::Release
        };
        let screen_open = self.mod_mappings_screen_open();
        let held = glfw_modifiers(self.modifiers);
        self.native_input
            .mappings
            .button(button, action, screen_open, held);
    }
    fn mod_mappings_input(&mut self, input: &str, action: KeyAction, mouse: bool) {
        let screen_open = self.mod_mappings_screen_open();
        let held = glfw_modifiers(self.modifiers);
        self.native_input
            .mappings
            .input(input, action, mouse, screen_open, held);
    }
    /// A screen or overlay is open: the mappings hear no presses and are released.
    pub(in crate::game) fn mod_mappings_screen_open(&self) -> bool {
        !gameplay_has_keyboard(
            self.screen,
            self.title.is_some(),
            self.screen_over_playing(),
        )
    }
    /// `KeyMapping.isDown`: held, while its conflict context and modifiers are active. Nothing is
    /// down behind a screen, which released them all.
    pub(in crate::game) fn is_down(&self, mod_id: &str, id: &str) -> bool {
        !self.mod_mappings_screen_open()
            && self
                .native_input
                .mappings
                .is_down(mod_id, id, glfw_modifiers(self.modifiers), false)
    }
    /// `KeyMapping.consumeClick`. Behind a screen there is none: the screen released them all.
    pub(in crate::game) fn consume_click(&mut self, mod_id: &str, id: &str) -> bool {
        if self.mod_mappings_screen_open() {
            self.native_input.mappings.release_all();
            return false;
        }
        self.native_input.mappings.consume_click(mod_id, id)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn opening_and_closing_a_direct_screen_discards_pending_clicks() {
        crate::game::native_entity_refs::tests::with_game(|game| {
            game.title = None;
            game.native_input
                .mappings
                .bind([horizon_client::keymapping::ModBinding {
                    owner: "test",
                    id: "pending",
                    input: "key.keyboard.k".into(),
                    modifiers: 0,
                    context: horizon_client::keymapping::MappingContext::InGame,
                }]);
            game.native_input.mappings.input(
                "key.keyboard.k",
                horizon_client::keymapping::KeyAction::Press,
                false,
                false,
                0,
            );
            game.open_emote_wheel();
            game.close_emote_wheel(None);
            assert!(!game.consume_click("test", "pending"));
        });
    }
}
