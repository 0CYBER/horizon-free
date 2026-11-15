//! The `screen_key_pressed_pre` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0108

pub trait Hooks {
    /// A key went down over the open screen (`ScreenEvent.KeyPressed.Pre`,
    /// vanilla's `Screen.keyPressed`): `screen` is the screen it is, `key`
    /// vanilla's name for the key (`key.keyboard.e`) — a key with no name
    /// fires nothing — `repeat` whether it is
    /// the keyboard's auto-repeat of a held press (Horizon's extra; NeoForge
    /// folds repeats into the event), and `modifiers` GLFW's bits (shift 1,
    /// control 2, alt 4, super 8). Set `canceled` and the press goes no
    /// further: no focus move, no Escape-close, no inventory or hotbar key, no
    /// typing by the key itself.
    ///
    /// It fires for the window's presses and their repeats, and for the
    /// control socket's `key` (a release fires nothing), after the window's
    /// own shortcuts have run — the fullscreen and screenshot keys, the debug
    /// keys and the narrator — and while a screen of the game's or the title's
    /// is open (Options and the pack list opened from the title screen count,
    /// with no player and no `menu`; the title's own). It does
    /// not fire on the connect and loading screens, nor for a server dialog, the server pack,
    /// link and code-of-conduct prompts, a book or a sign editor drawn over
    /// gameplay, nor while a world loads or saves: the key there stays the
    /// game's own. A cancelled press's release still runs, and its characters
    /// are GLFW's separate char callback and still come; see
    /// `screen_char_typed_pre`.
    #[hook(
        side = "client",
        contract = "cancel",
        out = "canceled",
        neoforge = "ScreenEvent.KeyPressed.Pre",
        cancellable = true
    )]
    fn screen_key_pressed_pre();
}
