//! The down/click state of `KeyMapping` and `ToggleKeyMapping` (sneak,
//! sprint, attack and use can be toggled instead of held).
use std::collections::HashMap;

/// The bound of a mapping's pending clicks: its count's largest value. Nothing in the engine
/// drains them in a loop; a mod's `consumeClick` takes one a call.
const MAPPING_CLICKS: crate::input_queue::QueueLimits = crate::input_queue::QueueLimits {
    name: "key mapping clicks",
    max_len: i32::MAX as usize,
    per_tick: 1,
};

/// One mapping's runtime state. `toggle` is `Some(restore)` for a
/// `ToggleKeyMapping` (with its `shouldRestore` flag), `None` for a plain one.
#[derive(Clone, Debug)]
pub struct KeyState {
    pub is_down: bool,
    click_count: i32,
    /// A click was refused at the largest count, and the log said so.
    click_refused: bool,
    released_by_screen_when_down: bool,
    toggle: Option<bool>,
    keyboard: bool,
    /// Held down by a controller's toggle (Horizon's pad toggle-sneak, which
    /// vanilla has no option for): until it lets go, the mapping toggles as
    /// if its vanilla option were on, so a screen releases and restores it.
    pad_toggled: bool,
}

impl KeyState {
    pub fn plain() -> Self {
        Self {
            is_down: false,
            click_count: 0,
            click_refused: false,
            released_by_screen_when_down: false,
            toggle: None,
            keyboard: true,
            pad_toggled: false,
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
    /// `KeyMapping.click`: one more pending click. At the largest count the click is refused
    /// instead of wrapping (vanilla's `clickCount++` would).
    pub fn click(&mut self) {
        let count = self.click_count.saturating_add(1);
        if count == self.click_count {
            crate::input_queue::refuse(&mut self.click_refused, MAPPING_CLICKS);
        }
        self.click_count = count;
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
    /// The mapping toggles: its option is on, or a pad toggled it down.
    fn needs_toggle(&self, option: bool) -> bool {
        option || self.pad_toggled
    }
    /// `setDown`; `needs_toggle` is the mapping's option value.
    pub fn set_down(&mut self, down: bool, needs_toggle: bool) {
        if self.toggle.is_some() && self.needs_toggle(needs_toggle) {
            if down {
                self.is_down = !self.is_down;
                // Toggled back up: the pad's toggle is over.
                self.pad_toggled &= self.is_down;
            }
        } else {
            self.is_down = down;
        }
    }
    /// `KeyboardHandler.keyPress` for this mapping's key: a release sets it up
    /// (`KeyMapping.set(key, false)`); a press or a repeat with no screen open
    /// sets it down and clicks it (`KeyMapping.set(key, true)`, `KeyMapping.click`).
    pub fn key(&mut self, action: KeyAction, screen_open: bool, needs_toggle: bool) {
        match action {
            KeyAction::Release => self.set_down(false, needs_toggle),
            KeyAction::Press | KeyAction::Repeat if !screen_open => {
                self.set_down(true, needs_toggle);
                self.click();
            }
            _ => {}
        }
    }
    /// `MouseHandler.onButton` for this mapping's button: only without a
    /// screen or overlay, and a press clicks it.
    pub fn button(&mut self, pressed: bool, screen_open: bool, needs_toggle: bool) {
        if !screen_open {
            self.set_down(pressed, needs_toggle);
            if pressed {
                self.click();
            }
        }
    }
    /// A controller's toggle button: flips the mapping like `setDown(true)`
    /// with its option on, whatever the option says.
    pub fn pad_toggle(&mut self) {
        self.is_down = !self.is_down;
        self.pad_toggled = self.is_down;
    }
    /// `shouldSetOnIngameFocus` for a keyboard-bound, bound key.
    pub fn should_set_on_ingame_focus(&self, needs_toggle: bool) -> bool {
        self.keyboard && (self.toggle.is_none() || !needs_toggle)
    }
    /// `release` (a screen opened): drops pending clicks and the held state.
    pub fn release(&mut self, needs_toggle: bool) {
        if self.toggle.is_some() {
            // ToggleKeyMapping.release does not clear pending clicks.
            if self.needs_toggle(needs_toggle) && self.is_down || self.released_by_screen_when_down
            {
                self.released_by_screen_when_down = true;
            }
            self.reset();
        } else {
            self.click_count = 0;
            self.set_down(false, needs_toggle);
        }
    }
    /// `ToggleKeyMapping.reset`. A pad toggle stays on record, so the screen
    /// that released it restores it.
    pub fn reset(&mut self) {
        self.is_down = false;
    }
    /// Clicks not yet consumed.
    pub fn pending_clicks(&self) -> i32 {
        self.click_count
    }
    /// `shouldRestoreStateOnScreenClosed` (clears the remembered release).
    pub fn should_restore_state_on_screen_closed(&mut self, needs_toggle: bool) -> bool {
        let Some(restore) = self.toggle else {
            return false;
        };
        let result = restore
            && self.needs_toggle(needs_toggle)
            && self.keyboard
            && self.released_by_screen_when_down;
        self.released_by_screen_when_down = false;
        result
    }
}

/// A keyboard event (`KeyEvent.Action`): SDL reports a held key's repeats
/// as their own action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyAction {
    Release,
    Press,
    Repeat,
}

/// Sneak.
pub const SNEAK: usize = 0;
/// Sprint.
pub const SPRINT: usize = 1;
/// Attack (mouse).
pub const ATTACK: usize = 2;

/// The `ToggleKeyMapping`s `Options` builds: sneak and sprint (keyboard,
/// restored after a screen), attack (mouse, restored) and use (mouse, not
/// restored), and the handlers that drive them. `needs_toggle` is each
/// mapping's toggle option, read at every event as vanilla does.
#[derive(Clone, Debug)]
pub struct ToggleKeys(pub [KeyState; 4]);
impl Default for ToggleKeys {
    fn default() -> Self {
        Self([
            KeyState::toggle(true, true),
            KeyState::toggle(true, true),
            KeyState::toggle(true, false),
            KeyState::toggle(false, false),
        ])
    }
}
impl ToggleKeys {
    pub fn is_down(&self, slot: usize) -> bool {
        self.0[slot].is_down
    }
    /// `KeyboardHandler.keyPress` for a key bound to `slot`: a release sets
    /// the mapping up (`KeyMapping.set(key, false)`); a press or a repeat
    /// with no screen open sets it down and clicks it.
    pub fn key(&mut self, slot: usize, action: KeyAction, screen_open: bool, needs_toggle: bool) {
        self.0[slot].key(action, screen_open, needs_toggle);
    }
    /// A controller's toggle button bound to `slot` (the pad's toggle-sneak):
    /// a press with no screen open flips the mapping and clicks it, as a
    /// key press does with the mapping's toggle option on.
    pub fn pad_toggle(&mut self, slot: usize, screen_open: bool) {
        if !screen_open {
            self.0[slot].pad_toggle();
            self.0[slot].click();
        }
    }
    /// `MouseHandler.onButton` for a button bound to `slot`: only without a
    /// screen or overlay.
    pub fn button(&mut self, slot: usize, pressed: bool, screen_open: bool, needs_toggle: bool) {
        self.0[slot].button(pressed, screen_open, needs_toggle);
    }
    /// `Gui.setScreen` with a screen: `KeyMapping.releaseAll`.
    pub fn screen_opened(&mut self, needs_toggle: [bool; 4]) {
        for (key, option) in self.0.iter_mut().zip(needs_toggle) {
            key.release(option);
        }
    }
    /// `Gui.setScreen(null)` in a level:
    /// `KeyMapping.restoreToggleStatesOnScreenClosed`.
    pub fn screen_closed(&mut self, needs_toggle: [bool; 4]) {
        for (key, option) in self.0.iter_mut().zip(needs_toggle) {
            if key.should_restore_state_on_screen_closed(option) {
                key.set_down(true, option);
            }
        }
    }
    /// `LocalPlayer.respawn`: `KeyMapping.resetToggleKeys`.
    pub fn respawn(&mut self) {
        for key in &mut self.0 {
            key.reset();
            // As with the vanilla option, the death screen closing after
            // this still restores a toggle it released.
            key.pad_toggled &= key.released_by_screen_when_down;
        }
    }
}

/// The screens a mod's mapping acts in (`KeyConflictContext`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MappingContext {
    Universal,
    InGame,
    Gui,
}

/// A mod's mapping as the player bound it, for [`ModKeys::bind`].
pub struct ModBinding<'a> {
    pub owner: &'a str,
    pub id: &'a str,
    /// The key's or mouse button's name (`key.keyboard.z`, `key.mouse.left`).
    pub input: String,
    /// The modifiers it holds, in GLFW's encoding (shift 1, control 2, alt 4).
    pub modifiers: u32,
    pub context: MappingContext,
}

#[derive(Clone, Debug)]
struct ModKey {
    state: KeyState,
    modifiers: u32,
    context: MappingContext,
}
impl ModKey {
    /// `isConflictContextAndModifierActive`: its context's screens are showing
    /// (in game with none open, a GUI with one) and every modifier its binding
    /// holds is down (`KeyModifier.isActive`).
    fn active(&self, held: u32, screen_open: bool) -> bool {
        let shown = match self.context {
            MappingContext::Universal => true,
            MappingContext::InGame => !screen_open,
            MappingContext::Gui => screen_open,
        };
        shown && self.modifiers & held == self.modifiers
    }
}

/// The plain `KeyMapping`s mods register (`Registrar::register_key_mapping`):
/// bound once at registration and again when a binding changes, with an index
/// from each key to its mappings, so an input touches only the mappings bound
/// to it and allocates nothing.
#[derive(Clone, Debug, Default)]
pub struct ModKeys {
    keys: Vec<ModKey>,
    /// Each bound key's mappings, by its name.
    by_input: HashMap<Box<str>, Vec<usize>>,
    by_mouse: HashMap<winit::event::MouseButton, Vec<usize>>,
    /// Each mod's mappings, by id.
    by_id: HashMap<Box<str>, HashMap<Box<str>, usize>>,
    /// Every mapping is up with no clicks, so `release_all` has nothing to do.
    idle: bool,
}
impl ModKeys {
    /// Binds the mappings, in registration order. A mapping keeps its held state
    /// and pending clicks across a rebinding; an unbound one hears no input.
    pub fn bind<'a>(&mut self, bindings: impl IntoIterator<Item = ModBinding<'a>>) {
        self.by_input.clear();
        self.by_mouse.clear();
        self.by_id.clear();
        let mut count = 0;
        for (index, binding) in bindings.into_iter().enumerate() {
            count = index + 1;
            let key = ModKey {
                state: KeyState::plain(),
                modifiers: binding.modifiers,
                context: binding.context,
            };
            match self.keys.get_mut(index) {
                Some(old) => {
                    old.modifiers = key.modifiers;
                    old.context = key.context;
                }
                None => self.keys.push(key),
            }
            if let Some(button) = crate::keybinds::mouse_button(&binding.input) {
                self.by_mouse.entry(button).or_default().push(index);
            }
            if !binding.input.is_empty() && binding.input != "key.keyboard.unknown" {
                self.by_input
                    .entry(binding.input.into_boxed_str())
                    .or_default()
                    .push(index);
            }
            self.by_id
                .entry(binding.owner.into())
                .or_default()
                .insert(binding.id.into(), index);
        }
        self.keys.truncate(count);
    }
    /// `KeyboardHandler.keyPress` (`mouse` false) or `MouseHandler.onButton`
    /// (`mouse` true) for the mappings bound to `input`: a press sets and
    /// clicks each one whose conflict context and modifiers are active
    /// (NeoForge's `isActiveAndMatches`), a release lets go of every one.
    /// `held` is the modifier word now.
    pub fn input(
        &mut self,
        input: &str,
        action: KeyAction,
        mouse: bool,
        screen_open: bool,
        held: u32,
    ) {
        let Some(indices) = self.by_input.get(input) else {
            return;
        };
        Self::dispatch(
            &mut self.keys,
            &mut self.idle,
            indices,
            action,
            mouse,
            screen_open,
            held,
        );
    }
    /// Dispatch a mouse event by the identity resolved at binding time.
    pub fn button(
        &mut self,
        button: winit::event::MouseButton,
        action: KeyAction,
        screen_open: bool,
        held: u32,
    ) {
        if let Some(indices) = self.by_mouse.get(&button) {
            Self::dispatch(
                &mut self.keys,
                &mut self.idle,
                indices,
                action,
                true,
                screen_open,
                held,
            );
        }
    }
    fn dispatch(
        keys: &mut [ModKey],
        idle: &mut bool,
        indices: &[usize],
        action: KeyAction,
        mouse: bool,
        screen_open: bool,
        held: u32,
    ) {
        for &index in indices {
            let key = &mut keys[index];
            if action != KeyAction::Release && !key.active(held, screen_open) {
                continue;
            }
            if mouse {
                key.state
                    .button(action != KeyAction::Release, screen_open, false);
            } else {
                key.state.key(action, screen_open, false);
            }
            *idle = false;
        }
    }
    fn index(&self, owner: &str, id: &str) -> Option<usize> {
        self.by_id.get(owner)?.get(id).copied()
    }
    /// `isDown` of mod `owner`'s mapping `id`: held, while its conflict context
    /// and modifiers are active.
    pub fn is_down(&self, owner: &str, id: &str, held: u32, screen_open: bool) -> bool {
        self.index(owner, id).is_some_and(|index| {
            let key = &self.keys[index];
            key.state.is_down && key.active(held, screen_open)
        })
    }
    /// `consumeClick` of mod `owner`'s mapping `id`.
    pub fn consume_click(&mut self, owner: &str, id: &str) -> bool {
        self.index(owner, id)
            .is_some_and(|index| self.keys[index].state.consume_click())
    }
    /// `KeyMapping.releaseAll` (a screen opened): every mapping is up and its
    /// pending clicks are gone.
    pub fn release_all(&mut self) {
        if self.idle {
            return;
        }
        for key in &mut self.keys {
            key.state.release(false);
        }
        self.idle = true;
    }
    /// Every mapping is up (a script let go of all its keys); pending clicks stay.
    pub fn set_all_up(&mut self) {
        for key in &mut self.keys {
            key.state.set_down(false, false);
        }
    }
}

#[cfg(test)]
mod tests {
    fn binding(
        id: &'static str,
        input: &str,
        modifiers: u32,
        context: MappingContext,
    ) -> ModBinding<'static> {
        ModBinding {
            owner: "m",
            id,
            input: input.to_owned(),
            modifiers,
            context,
        }
    }
    /// `zoom` on Z, `boost` on Shift + Z in game, `grab` on the left button.
    fn keys() -> ModKeys {
        let mut keys = ModKeys::default();
        keys.bind([
            binding("zoom", "key.keyboard.z", 0, MappingContext::Universal),
            binding("boost", "key.keyboard.z", 1, MappingContext::InGame),
            binding("grab", "key.mouse.left", 0, MappingContext::Universal),
        ]);
        keys
    }
    /// A tap is one click, consumed once.
    #[test]
    fn a_tap_adds_one_click() {
        let mut keys = keys();
        keys.input("key.keyboard.z", KeyAction::Press, false, false, 0);
        assert!(keys.is_down("m", "zoom", 0, false));
        keys.input("key.keyboard.z", KeyAction::Release, false, false, 0);
        assert!(!keys.is_down("m", "zoom", 0, false));
        assert!(keys.consume_click("m", "zoom"));
        assert!(!keys.consume_click("m", "zoom"));
        assert!(!keys.consume_click("m", "boost"), "shift was up");
        assert!(!keys.consume_click("m", "grab"));
        assert!(!keys.consume_click("other", "zoom"));
    }
    /// Each auto-repeat of a held key is one more click, as its press.
    #[test]
    fn a_held_key_adds_one_click_per_repeat() {
        let mut keys = keys();
        keys.input("key.keyboard.z", KeyAction::Press, false, false, 1);
        keys.input("key.keyboard.z", KeyAction::Repeat, false, false, 1);
        keys.input("key.keyboard.z", KeyAction::Repeat, false, false, 1);
        assert!(keys.is_down("m", "boost", 1, false));
        assert!(!keys.is_down("m", "boost", 0, false), "shift let go");
        assert_eq!(
            (0..4)
                .map(|_| keys.consume_click("m", "boost"))
                .collect::<Vec<_>>(),
            [true, true, true, false]
        );
    }
    /// A screen opening drops every pending click and lets go; presses behind
    /// it count nothing, its releases still let go.
    #[test]
    fn a_screen_opening_zeroes_the_clicks() {
        let mut keys = keys();
        keys.input("key.keyboard.z", KeyAction::Press, false, false, 0);
        keys.input("key.keyboard.z", KeyAction::Repeat, false, false, 0);
        keys.input("key.mouse.left", KeyAction::Press, true, false, 0);
        keys.release_all();
        assert!(!keys.is_down("m", "zoom", 0, true) && !keys.is_down("m", "grab", 0, true));
        assert!(!keys.consume_click("m", "zoom") && !keys.consume_click("m", "grab"));
        keys.input("key.keyboard.z", KeyAction::Press, false, true, 0);
        keys.input("key.mouse.left", KeyAction::Press, true, true, 0);
        assert!(!keys.consume_click("m", "zoom") && !keys.consume_click("m", "grab"));
    }
    /// A rebinding moves a mapping to its new key and keeps its pending clicks.
    #[test]
    fn a_rebinding_moves_the_mapping() {
        let mut keys = keys();
        keys.input("key.keyboard.z", KeyAction::Press, false, false, 0);
        keys.bind([binding(
            "zoom",
            "key.keyboard.x",
            0,
            MappingContext::Universal,
        )]);
        keys.input("key.keyboard.z", KeyAction::Press, false, false, 0);
        keys.input("key.keyboard.x", KeyAction::Press, false, false, 0);
        assert_eq!(
            (0..3)
                .map(|_| keys.consume_click("m", "zoom"))
                .collect::<Vec<_>>(),
            [true, true, false]
        );
        assert!(!keys.consume_click("m", "boost"), "no longer registered");
    }
    /// `KeyMapping.setDown` holds without a click; `KeyMapping.click` is a press's increment.
    #[test]
    fn set_changes_only_the_held_state() {
        let mut key = KeyState::plain();
        key.set_down(true, false);
        assert!(key.is_down);
        assert!(!key.consume_click());
        key.click();
        assert!(key.is_down);
        assert!(key.consume_click());
        assert!(!key.consume_click());
    }

    /// `KeyMapping.resetToggleKeys` (a new `LocalPlayer`): a sneak and a sprint toggled on in one
    /// world are off in the next.
    #[test]
    fn resetting_the_toggle_keys_puts_every_mapping_up() {
        let mut keys = ToggleKeys::default();
        keys.key(0, KeyAction::Press, false, true);
        keys.key(1, KeyAction::Press, false, true);
        assert!(keys.is_down(0) && keys.is_down(1));
        for key in &mut keys.0 {
            key.reset();
        }
        assert!(!keys.is_down(0) && !keys.is_down(1));
    }

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
    fn screens_block_presses_but_not_releases() {
        let mut keys = ToggleKeys::default();
        keys.key(SNEAK, KeyAction::Press, true, false);
        assert!(!keys.is_down(SNEAK));
        keys.key(SNEAK, KeyAction::Press, false, false);
        assert!(keys.is_down(SNEAK));
        keys.key(SNEAK, KeyAction::Release, true, false);
        assert!(!keys.is_down(SNEAK));
        keys.key(SPRINT, KeyAction::Press, false, true);
        keys.respawn();
        assert!(!keys.is_down(SPRINT));
    }
    /// #3061: the pad's toggle-sneak (vanilla's toggle crouch off) comes
    /// back after a screen, as the vanilla option's toggle does; a
    /// keyboard shift-click in the inventory does not drop it.
    #[test]
    fn pad_toggle_is_restored_after_a_screen() {
        let off = [false; 4];
        let mut keys = ToggleKeys::default();
        keys.pad_toggle(SNEAK, false);
        assert!(keys.is_down(SNEAK));
        keys.screen_opened(off);
        assert!(!keys.is_down(SNEAK));
        keys.key(SNEAK, KeyAction::Press, true, false);
        keys.key(SNEAK, KeyAction::Release, true, false);
        keys.screen_closed(off);
        assert!(keys.is_down(SNEAK), "the pad's toggle is restored");
        // Toggled back up, it stays up through the next screen.
        keys.pad_toggle(SNEAK, false);
        assert!(!keys.is_down(SNEAK));
        keys.screen_opened(off);
        keys.screen_closed(off);
        assert!(!keys.is_down(SNEAK));
        // A held key is not restored, as before.
        keys.key(SNEAK, KeyAction::Press, false, false);
        keys.screen_opened(off);
        keys.key(SNEAK, KeyAction::Release, true, false);
        keys.screen_closed(off);
        assert!(!keys.is_down(SNEAK));
    }
    /// After the pad's toggle is over (toggled up, or reset by a respawn
    /// with no screen to restore it), the shift key holds again.
    #[test]
    fn pad_toggle_ends_with_its_release() {
        let off = [false; 4];
        let mut keys = ToggleKeys::default();
        keys.pad_toggle(SNEAK, false);
        keys.respawn();
        assert!(!keys.is_down(SNEAK));
        keys.key(SNEAK, KeyAction::Press, false, false);
        keys.key(SNEAK, KeyAction::Release, false, false);
        assert!(!keys.is_down(SNEAK), "held, not toggled");
        // The death screen releases it and closes after the respawn: the
        // toggle comes back, as with the vanilla option.
        keys.pad_toggle(SNEAK, false);
        keys.screen_opened(off);
        keys.respawn();
        keys.screen_closed(off);
        assert!(keys.is_down(SNEAK));
        // A press with a screen open does nothing.
        keys.pad_toggle(SNEAK, true);
        assert!(keys.is_down(SNEAK));
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

    /// A mapping's pending clicks stop at the largest count instead of wrapping.
    #[test]
    fn pending_clicks_stop_at_the_largest_count() {
        let mut key = KeyState::plain();
        key.click_count = i32::MAX - 1;
        key.click();
        key.click();
        assert_eq!(key.pending_clicks(), i32::MAX);
        assert!(key.consume_click());
        assert_eq!(key.pending_clicks(), i32::MAX - 1);
    }
}
