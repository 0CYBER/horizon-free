//! The `screen_mouse_button_pressed_pre` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0093

pub trait Hooks {
    /// A mouse button went down over the open screen
    /// (`ScreenEvent.MouseButtonPressed.Pre`, vanilla's
    /// `Screen.mouseClicked`): `screen` is the screen it is, `screen.mouse`
    /// where the pointer is (so there is no position of its own), `button` the
    /// button's GLFW index (0 left, 1 right, 2 middle, 3 back, 4 forward; a
    /// button past those is the platform's own number, which differs between
    /// systems) and `modifiers` GLFW's bits (shift 1, control 2,
    /// alt 4, super 8). Set `canceled` and the press goes no further: no slot,
    /// no screen button, no drag, no hotbar key, no sort or header button.
    ///
    /// It fires before anything the screen does with the press, on the window
    /// only, and while a screen of the game's or the title's is open (Options
    /// and the pack list opened from the title screen count, with no player
    /// and no `menu`; the title's own screens, their
    /// `screen.id` from the title state machine). It does not fire on the
    /// connect and loading screens, nor for a server
    /// dialog, the server pack, link and code-of-conduct prompts, a book or a
    /// sign editor drawn over gameplay, nor while a world loads or saves: the
    /// click there stays the game's own. A gamepad click or a click the
    /// keyboard's focus sends is not the mouse and fires nothing.
    #[hook(
        side = "client",
        contract = "cancel",
        out = "canceled",
        neoforge = "ScreenEvent.MouseButtonPressed.Pre",
        cancellable = true
    )]
    fn screen_mouse_button_pressed_pre();
}
