//! The `key_mapping_pre` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A physical key or mouse button bound to the gameplay mapping `mapping` (`key.jump`,
    /// `key.attack`...) changes state while playing, before the game acts on it. `pressed` is true
    /// for a press (a key's repeat counts as one, as the game takes it) and false for a release.
    /// Cancel a press to drop it for every mapping bound to its input: the game holds nothing and
    /// acts on nothing. A release cannot be canceled, so what the game holds is always let go.
    /// Told for the keys and mouse buttons the window takes (agent control's included), after the
    /// window's own shortcuts (fullscreen, screenshot, debug keys), and only with no screen open;
    /// not for a controller, whose presses go straight to the mappings, nor for a mod's
    /// `set_mapping_down` and `click_mapping`, which are no physical input. No NeoForge counterpart.
    /// Client-side.
    #[hook(
        side = "client",
        contract = "cancel",
        out = "canceled",
        neoforge = "InputEvent.InteractionKeyMappingTriggered, InputEvent.Key"
    )]
    fn key_mapping_pre();
}
