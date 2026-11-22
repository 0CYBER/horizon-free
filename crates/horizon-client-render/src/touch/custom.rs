// Adapted from Starling (https://github.com/starling-client/starling, `android` branch at
// 150a414b09a1, `crates/horizon-client-render/src/touch/custom.rs`), written by Vasysik and the
// Starling contributors. Copyright (c) 2026 Ludovic Marie and the Horizon contributors, under the
// same MIT license as Horizon (LICENSE at the repository root).

//! Buttons the player adds. Each sends a key (`key.keyboard.z`) or a game
//! action (`action:key.jump`) through the game's key bindings, held while any
//! finger is on it. With the inventory open, the sneak, attack and use buttons
//! send shift and the mouse buttons instead, so items can be shift-clicked and
//! dragged.
use super::{Control, Role, TouchControls, draw, layout};
use crate::hud::{HudAtlas, HudBatch, rgba};
use layout::Placement;
use serde::{Deserialize, Serialize};

/// The token of a button that has no key yet: it sends nothing.
pub const UNASSIGNED: &str = "key.keyboard.unknown";

/// A button the player added.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CustomButton {
    /// `key.keyboard.*`, `action:` and a key mapping name, or [`UNASSIGNED`].
    pub key: String,
    /// Where it sits in each [`Mode`](super::Mode).
    pub layouts: [Placement; 3],
}

impl CustomButton {
    /// A new button; `index` spreads new buttons over a grid right of centre.
    pub fn new(key: String, index: usize) -> Self {
        let place = Placement::new(
            0.68 + (index % 3) as f32 * 0.1,
            0.35 + (index / 3 % 3) as f32 * 0.12,
            0.105,
        );
        Self {
            key,
            layouts: [place; 3],
        }
    }

    /// Whether a token read from disk names a key or a key mapping.
    pub fn valid_token(token: &str) -> bool {
        token == UNASSIGNED
            || crate::keybinds::key_code(token).is_some()
            || token
                .strip_prefix("action:")
                .is_some_and(|name| crate::keybinds::all_mappings().any(|m| m.0 == name))
    }

    /// What the button shows: the key's name or the action's translated name.
    pub fn label(&self) -> String {
        if self.key == UNASSIGNED {
            return crate::lang::get("horizon.touch.button");
        }
        if let Some(name) = self.key.strip_prefix("action:") {
            return crate::text::Text::translate(name, &[]).plain();
        }
        crate::keybinds::display_name(&self.key).plain()
    }
}

impl TouchControls {
    /// The key a built-in button sends while the inventory is open.
    pub(super) fn inventory_token(control: Control) -> Option<&'static str> {
        match control {
            Control::Sneak => Some("key.keyboard.left.shift"),
            Control::Attack => Some("action:key.attack"),
            Control::Use => Some("action:key.use"),
            _ => None,
        }
    }

    /// A touch while the inventory is open: true when the sneak, attack or use
    /// button (or a custom button) took it. The rest belong to the inventory.
    pub fn inventory_update(
        &mut self,
        id: u64,
        point: [f32; 2],
        started: bool,
        ended: bool,
    ) -> bool {
        // A finger already on a custom button, or a new one landing on an
        // assigned custom button, presses its key here too.
        if self.custom_update(id, point, started, ended) {
            return true;
        }
        if started
            && self.custom_at(point).is_none()
            && let Some(token) = self.control_at(point).and_then(Self::inventory_token)
        {
            self.hold(id, point, token.into());
            return true;
        }
        false
    }

    /// Custom button `index` on this surface, mirrored and scaled like the rest.
    pub(super) fn custom_placement(&self, index: usize) -> Placement {
        let mut p = self.settings.custom_buttons[index].layouts[self.settings.mode.index()];
        if self.settings.left_handed {
            p.center[0] = 1.0 - p.center[0];
        }
        self.fit(p)
    }

    /// The custom button under `point`, the last-drawn one first.
    pub fn custom_at(&self, point: [f32; 2]) -> Option<usize> {
        (0..self.settings.custom_buttons.len())
            .rev()
            .find(|&i| layout::contains(self.custom_placement(i).rect(self.aspect()), point))
    }

    /// Whether any button, built-in or custom, is under `point`.
    pub fn control_at_point(&self, point: [f32; 2]) -> bool {
        self.custom_at(point).is_some() || self.control_at(point).is_some()
    }

    /// Presses `token` for finger `id`; a key already held by another finger
    /// is not pressed twice.
    fn hold(&mut self, id: u64, point: [f32; 2], token: String) {
        if !self.custom_fingers.values().any(|k| *k == token) {
            self.key_events.push((token.clone(), true));
        }
        self.custom_fingers.insert(id, token);
        self.fingers.insert(id, (Role::Ignore, point));
    }

    /// True when a custom button owns this touch. A key is released when its
    /// last finger lifts, wherever that finger has moved to.
    pub(super) fn custom_update(
        &mut self,
        id: u64,
        point: [f32; 2],
        started: bool,
        ended: bool,
    ) -> bool {
        if let Some(token) = self.custom_fingers.get(&id).cloned() {
            if ended {
                self.custom_fingers.remove(&id);
                self.fingers.remove(&id);
                if !self.custom_fingers.values().any(|k| *k == token) {
                    self.key_events.push((token, false));
                }
            }
            return true;
        }
        if started && let Some(index) = self.custom_at(point) {
            let token = self.settings.custom_buttons[index].key.clone();
            if token == UNASSIGNED {
                return false;
            }
            self.hold(id, point, token);
            self.clicked = true;
            return true;
        }
        false
    }

    /// The keys and actions custom fingers hold now.
    pub fn held_custom_tokens(&self) -> impl Iterator<Item = &str> {
        self.custom_fingers.values().map(String::as_str)
    }

    /// Releases every held custom key (each once).
    pub(super) fn clear_custom(&mut self) {
        let held: std::collections::BTreeSet<_> = std::mem::take(&mut self.custom_fingers)
            .into_values()
            .collect();
        for token in held {
            self.key_events.push((token, false));
        }
    }

    /// Drops the world gestures (the inventory opened) but keeps custom keys held.
    pub fn clear_world_gesture(&mut self) {
        self.fingers
            .retain(|_, (role, _)| matches!(role, Role::Ignore));
        self.sprint_latched = false;
        self.pending = Default::default();
        self.aim = None;
        self.tap_aim = None;
        self.pending_tap = None;
        self.last_marker = None;
        self.holding = false;
        self.holding_use = false;
    }

    pub(super) fn draw_custom(&self, b: &mut HudBatch, a: &HudAtlas) {
        for (i, button) in self.settings.custom_buttons.iter().enumerate() {
            let p = self.custom_placement(i);
            if p.opacity <= 0.0 {
                continue;
            }
            let pressed = self.custom_fingers.values().any(|k| *k == button.key);
            let color = rgba(255, 255, 255, draw::alpha(p.opacity));
            let bounds = draw::square(b, p.center, p.size);
            draw::button(b, a, bounds, &button.label(), pressed, color);
        }
    }
}
