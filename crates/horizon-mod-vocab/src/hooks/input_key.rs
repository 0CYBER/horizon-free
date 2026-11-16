//! The `input_key` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0125

pub trait Hooks {
    /// A key event after handling, the window's or the control socket's `key` (one press, then
    /// one release for a tap), including repeats and consumed shortcuts (`InputEvent.Key`). `key` is vanilla's name for the physical key (`key.keyboard.a`,
    /// `key.keyboard.unknown` for one it doesn't name; `getKey`), `layout_key` its name
    /// for the key the keyboard layout produced (`getKeycode`: AZERTY's key at QWERTY's
    /// W gives `key.keyboard.z`; a named key such as Enter keeps the physical name;
    /// `None` for a character other than one ASCII letter or digit, a dead key's accent
    /// included since it arrives as its character, and for a key vanilla doesn't name)
    /// and `raw_code` a code telling physical keys apart, named or not (`getScanCode`:
    /// the platform scan code, stable within a platform only). `action` is what it did, and the modifier bits
    /// the screen hooks' GLFW encoding. Not cancellable.
    #[hook(
        side = "client",
        contract = "listen",
        version = 2,
        neoforge = "InputEvent.Key"
    )]
    fn input_key();
}
