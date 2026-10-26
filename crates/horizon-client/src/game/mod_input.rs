//! Capability `input` (docs/mods.md): the use button on items a mod claims,
//! and the mod's own keys. With no mod claiming items or keys, each hook
//! here returns after one check.
use super::Game;
use winit::{event::MouseButton, keyboard::KeyCode};
#[cfg(feature = "mods")]
use {super::Screen, horizon_mods::HeldItem};

/// A claimed item's use while the button is down.
#[cfg(feature = "mods")]
pub(super) struct ModUse {
    item: HeldItem,
    /// `Game::ticks` when it started.
    since: u64,
}

#[cfg(not(feature = "mods"))]
impl Game<'_> {
    pub(super) fn mod_use(&mut self, _button: MouseButton, _pressed: bool) -> bool {
        false
    }
    pub(super) fn mod_key(&mut self, _key: KeyCode, _pressed: bool, _repeat: bool) {}
}

#[cfg(feature = "mods")]
impl Game<'_> {
    /// The main hand's stack as `input` shows it; none when it is empty.
    pub(super) fn mod_held(&self) -> Option<HeldItem> {
        let stack = self.inventory.get(self.selected)?.as_ref()?;
        Some(HeldItem {
            id: stack.item.to_string(),
            count: stack.count,
            slot: self.selected as u32,
        })
    }
    /// The use button while playing: true when a mod claimed the held item,
    /// so the game does nothing else with it (no placing, eating, swing).
    pub(super) fn mod_use(&mut self, button: MouseButton, pressed: bool) -> bool {
        if button != MouseButton::Right || !self.mods.mods.claims_any() {
            return false;
        }
        if !pressed {
            return self.stop_mod_use();
        }
        let Some(item) = self.mod_held().filter(|h| self.mods.mods.claims(&h.id)) else {
            return false;
        };
        self.mods.mods.use_start(&item);
        self.mods.using = Some(ModUse {
            item,
            since: self.ticks,
        });
        true
    }
    /// Ends a claimed use: the button came up, the player switched slots,
    /// or a screen opened. False when none was going.
    pub(super) fn stop_mod_use(&mut self) -> bool {
        let Some(u) = self.mods.using.take() else {
            return false;
        };
        let ticks = self.ticks.saturating_sub(u.since).min(u64::from(u32::MAX)) as u32;
        self.mods.mods.use_stop(&u.item, ticks);
        true
    }
    /// Once a tick: a claimed use stops when its item leaves the hand or a
    /// screen opens, as vanilla's `releaseUsingItem`.
    pub(super) fn tick_mod_use(&mut self) {
        let Some(u) = &self.mods.using else {
            return;
        };
        let still = self
            .mod_held()
            .is_some_and(|h| h.id == u.item.id && h.slot == u.item.slot);
        if !still || self.screen != Screen::Playing {
            self.stop_mod_use();
        }
    }
    /// A key went down or up: a mod's registered key while playing (no
    /// screen) is its `key-down` / `key-up`. Vanilla's mapping on the same
    /// key still runs.
    pub(super) fn mod_key(&mut self, key: KeyCode, pressed: bool, repeat: bool) {
        if repeat || !self.mods.mods.has_keys() {
            return;
        }
        if pressed && (self.screen != Screen::Playing || self.title.is_some()) {
            return;
        }
        if let Some(name) = horizon_client::keybinds::key_name(key) {
            self.mods.mods.key(name, pressed);
        }
    }
}
