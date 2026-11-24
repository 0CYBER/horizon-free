use super::*;
use horizon_client::{
    gamepad::{self, Button, GlyphOf, GlyphStyle},
    pad_glyph::{self, Glyph, Glyphs},
};
use mod_api::{ControllerButton, ControllerGlyphFamily, InputGlyph};

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

    pub(in crate::game) fn input_glyph(&self, action: &str) -> Option<InputGlyph> {
        let &(action, _, _) = keybinds::MAPPINGS.iter().find(|m| m.0 == action)?;
        if !self.controller_driving_glyphs() {
            return self.key_input_glyph(self.settings.keys.get(action));
        }
        // Not every vanilla mapping is available on a pad. In particular, stick movement
        // is an axis, not a button binding, and a stick-click icon would be misleading.
        gamepad::ACTIONS.iter().find(|m| m.0 == action)?;
        let settings = &self.settings.controller;
        let glyphs = Glyphs::of(settings, self.pad.style);
        Some(self.pad_input_glyph(Glyph::of(GlyphOf::Bound(action), settings, glyphs)?))
    }

    pub(in crate::game) fn mod_input_glyph(&self, mod_id: &str, id: &str) -> Option<InputGlyph> {
        if self.controller_driving_glyphs() {
            return None;
        }
        let (key, modifiers) = self.native_input.mappings.binding(mod_id, id)?;
        self.key_binding_glyph(key, modifiers)
    }

    pub(in crate::game) fn key_input_glyph(&self, key: &str) -> Option<InputGlyph> {
        self.key_binding_glyph(key, 0)
    }

    pub(in crate::game) fn controller_input_glyph(
        &self,
        button: ControllerButton,
    ) -> Option<InputGlyph> {
        if !self.settings.controller.enabled || !(self.pad.device.is_some() || self.pad.in_use) {
            return None;
        }
        // Before anyone has touched a connected controller, `style` need not name it yet.
        let detected = if self.pad.in_use {
            self.pad.style
        } else {
            self.pad.device.as_ref().map_or(self.pad.style, |device| {
                GlyphStyle::of_device(device.vendor, device.product)
            })
        };
        let family = self.settings.controller.glyphs.resolve(detected);
        Some(self.pad_input_glyph(Glyph::physical(engine_button(button)?, family)))
    }

    fn controller_driving_glyphs(&self) -> bool {
        self.settings.controller.enabled && self.pad.in_use && !self.pad.keyboard_took_over
    }

    fn key_binding_glyph(&self, key: &str, modifiers: u32) -> Option<InputGlyph> {
        if keybinds::key_code(key).is_none() && keybinds::mouse_button(key).is_none() {
            return None;
        }
        let label = if modifiers == 0 {
            keybinds::display_name(key)
        } else {
            horizon_client::options_ui::binding_text(key, modifiers)
        }
        .plain();
        let width = pad_glyph::key_label_width(&self.gui.atlas, &label);
        Some(mod_api::__private::game::input_glyph(
            label,
            [width, pad_glyph::HEIGHT],
            None,
        ))
    }

    fn pad_input_glyph(&self, glyph: Glyph) -> InputGlyph {
        let (family, button) = glyph.art_input();
        let family = match family {
            GlyphStyle::Auto | GlyphStyle::Xbox => ControllerGlyphFamily::Xbox,
            GlyphStyle::PlayStation => ControllerGlyphFamily::PlayStation,
            GlyphStyle::Nintendo => ControllerGlyphFamily::Nintendo,
        };
        mod_api::__private::game::input_glyph(
            glyph.label().into(),
            [pad_glyph::width(&self.gui.atlas, glyph), pad_glyph::HEIGHT],
            Some((family, api_button(button))),
        )
    }

    pub(in crate::game) fn native_input_glyph_draw(
        &self,
        batch: &mut HudBatch,
        glyph: &InputGlyph,
        [x, y]: [f32; 2],
        argb: u32,
    ) -> bool {
        if !x.is_finite() || !y.is_finite() {
            return false;
        }
        let [w, h] = glyph.size();
        if !(w.is_finite() && w > 0.0 && h == pad_glyph::HEIGHT) {
            return false;
        }
        let rect = [x, y, w, h];
        let [a, r, g, b] = argb.to_be_bytes();
        let tint = rgba(r, g, b, a);
        let atlas = &self.gui.atlas;
        if let Some((family, button)) = glyph.controller() {
            let family = match family {
                ControllerGlyphFamily::Xbox => GlyphStyle::Xbox,
                ControllerGlyphFamily::PlayStation => GlyphStyle::PlayStation,
                ControllerGlyphFamily::Nintendo => GlyphStyle::Nintendo,
                _ => return false,
            };
            let Some(button) = engine_button(button) else {
                return false;
            };
            pad_glyph::draw_tinted(batch, atlas, Glyph::physical(button, family), rect, tint);
        } else {
            pad_glyph::draw_key_label(batch, atlas, glyph.label(), rect, [255; 3], tint);
        }
        true
    }
}

// Keep positional identities explicit across the engine/SDK boundary; enum discriminants
// are not engine button numbers.
macro_rules! controller_buttons {
    ($($button:ident),+ $(,)?) => {
        fn api_button(button: Button) -> ControllerButton {
            match button {
                $(Button::$button => ControllerButton::$button,)+
            }
        }

        fn engine_button(button: ControllerButton) -> Option<Button> {
            Some(match button {
                $(ControllerButton::$button => Button::$button,)+
                _ => return None,
            })
        }
    };
}

controller_buttons!(
    South,
    East,
    West,
    North,
    LeftBumper,
    RightBumper,
    LeftTrigger,
    RightTrigger,
    Select,
    Start,
    LeftStick,
    RightStick,
    DPadUp,
    DPadDown,
    DPadLeft,
    DPadRight,
);
