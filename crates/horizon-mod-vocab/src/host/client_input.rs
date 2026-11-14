//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The client host's methods of the `client_input` area, implemented by the game and reached through [`crate::ClientHost`].
#[horizon_mod_api_macros::host_area]
pub trait ClientInputClientHost {
    /// Side: client.
    /// Right-clicks with these items (at most 64 ids) are mod `mod_id`'s; a later call replaces the list.
    #[access(WorldOnly)]
    fn claim_items(&mut self, mod_id: &str, ids: &[String]);
    /// Side: client.
    /// Mod `mod_id`'s events since its last poll (at most 256 wait).
    #[access(WorldOnly)]
    fn poll(&mut self, mod_id: &str) -> Vec<ModEvent>;
    /// Side: client.
    /// Where the mouse is, in GUI pixels.
    #[access(ClientSetup)]
    fn cursor(&self) -> [f32; 2];
    /// Side: client.
    /// The saved key and modifiers for an owner-bound registration, or its declared default.
    #[access(ClientSetup)]
    fn mod_key(&self, mod_id: &str, name: &str) -> Option<(String, ClientModifiers)>;
    /// Side: client.
    /// Whether mod `mod_id`'s registered mapping `id` is held (`KeyMapping.isDown`): its key or
    /// mouse button is down and its conflict context and modifiers are active. False for an
    /// unregistered id.
    #[access(ClientSetup)]
    fn is_down(&self, mod_id: &str, id: &str) -> bool;
    /// Side: client.
    /// Takes one click off mod `mod_id`'s registered mapping `id` (`KeyMapping.consumeClick`):
    /// true when it had one. A press, each auto-repeat while its key is held, and a mouse press
    /// add one, with no screen open and its conflict context and modifiers active, from the
    /// window or the control socket alike; a screen opening drops them all. False for an
    /// unregistered id.
    #[access(ClientSetup)]
    fn consume_click(&mut self, mod_id: &str, id: &str) -> bool;
    /// Side: client.
    /// Playing, with nothing over the world: keys and zoom apply.
    #[access(ClientSetup)]
    fn playing(&self) -> bool;
    /// Side: client.
    /// A vanilla mapping (`key.forward`, `key.jump`, `key.attack`…): whether
    /// it is held (`KeyMapping.isDown`), and its key's name as Controls shows
    /// it. `Ok(None)` for a mapping it doesn't answer for. Refuses an invalid
    /// label with [`TextFormError`], including components deeper than 512 nodes.
    #[access(ClientSetup)]
    fn mapping(&self, name: &str) -> Result<Option<KeyMapping>, TextFormError>;
    /// Side: client.
    /// Holds the vanilla gameplay mapping `key` down or lets it go (`KeyMapping.setDown`): the
    /// held state its key gives it, without the click a press adds. `key` is a mapping of the
    /// movement, gameplay or inventory category (`key.attack`, `key.use`, `key.sprint`,
    /// `key.forward`, `key.hotbar.1`...); a toggled mapping (sneak, sprint, attack, use with their
    /// toggle option on) flips on `true`, as `ToggleKeyMapping.setDown` does. The client tick's
    /// key step acts on it with all its checks: a held use repeats every four ticks, a held attack
    /// goes on breaking the block. With a screen or overlay open a press does nothing, and opening
    /// one lets go of every mapping, as `KeyMapping.releaseAll` does. `key_mapping_pre` is not
    /// asked: it is about physical keys.
    #[access(WorldOnly)]
    fn set_mapping_down(&mut self, key: &str, down: bool) -> Result<(), MappingError>;
    /// Side: client.
    /// Adds one click to the vanilla gameplay mapping `key` (`KeyMapping.click`), as a press of
    /// its key does: the attack, use or pick a click starts on the next client tick, through the
    /// checks of `startAttack` and `startUseItem` (the miss cooldown, busy hands, the item, the
    /// target under the crosshair) and the packets the player's own click sends; the hotbar slot,
    /// drop, inventory or hand swap of the inventory keys. With a screen or overlay open the click
    /// is dropped, as `KeyMapping.releaseAll` drops it. `interaction_key_mapping_triggered` fires
    /// at the action as for the player's click; `key_mapping_pre` is not asked.
    #[access(WorldOnly)]
    fn click_mapping(&mut self, key: &str) -> Result<(), MappingError>;
    /// Side: client.
    /// Milliseconds on the game's own clock (`Util.getMillis`), counting from the game's start:
    /// what a text field's cursor blinks by and what tells a double click from two clicks.
    /// Only differences between two readings mean anything.
    #[access(ClientSetup)]
    fn millis(&self) -> u64;
    /// Side: client.
    /// The current window modifier keys in the same GLFW encoding as input hooks.
    /// Cleared when the window loses focus; available even outside a world.
    #[access(ClientSetup)]
    fn modifiers(&self) -> ClientModifiers;
    /// Side: client.
    /// The clipboard's text (`KeyboardHandler.getClipboard`), empty when it holds none,
    /// while the game handles a key, mouse or typed-text event: a mod's key or screen hook
    /// pasting into its own field. None at any other moment (a tick, a frame, a packet),
    /// so no mod reads the clipboard without the player doing something, and None on a
    /// server.
    #[access(ClientSetup)]
    fn clipboard_text(&self) -> Option<String>;
    /// Side: client.
    /// Puts `text` on the clipboard (`KeyboardHandler.setClipboard`) on the same terms as
    /// `native_clipboard_text` reads it. False, with nothing copied, at any other moment or
    /// on a server; empty text copies nothing, as vanilla's.
    #[access(ClientSetup)]
    fn set_clipboard_text(&self, text: &str) -> bool;
}
