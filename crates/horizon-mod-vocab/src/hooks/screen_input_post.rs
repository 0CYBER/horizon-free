//! The `screen_input_post` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A mouse button went down over the open screen and the screen has handled it (`ScreenEvent.MouseButtonPressed.Post`, vanilla's `Screen.mouseClicked` returning): `screen`, `button` and `modifiers` as in `screen_mouse_button_pressed_pre`. Not cancellable. NeoForge's event also carries `wasClickHandled` / `wasReleaseHandled`; this one has no such field, because Horizon's screens take their input in their own arms and report nothing back. There is no double-click flag: Horizon does not track double clicks. Fires on the window only, while a screen of the game's or the title's is open (the screens the `_pre` input hooks cover), for real mouse or keyboard input and not a pad's or the keyboard focus's, and only when the matching Pre did not cancel it. It fires on the screen open after the screen's arms ran, so a press that closed the screen fires nothing.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "ScreenEvent.MouseButtonPressed.Post",
        cancellable = false
    )]
    fn screen_mouse_button_pressed_post();
    /// A mouse button went up over the open screen and the screen has handled it (`ScreenEvent.MouseButtonReleased.Post`, vanilla's `Screen.mouseReleased` returning), with the fields of `screen_mouse_button_released_pre`. Not cancellable. NeoForge's event also carries `wasClickHandled` / `wasReleaseHandled`; this one has no such field, because Horizon's screens take their input in their own arms and report nothing back. Fires on the window only, while a screen of the game's or the title's is open (the screens the `_pre` input hooks cover), for real mouse or keyboard input and not a pad's or the keyboard focus's, and only when the matching Pre did not cancel it. It fires on the screen open after the screen's arms ran, so a release that closed the screen fires nothing.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "ScreenEvent.MouseButtonReleased.Post",
        cancellable = false
    )]
    fn screen_mouse_button_released_post();
    /// A held-button pointer movement has gone through the open screen's drag handlers (`ScreenEvent.MouseDragged.Post`, vanilla's `Screen.mouseDragged`): `button`, `delta` (GUI pixels, `screen.mouse` the new position) and `modifiers` as in `screen_mouse_dragged_pre`. Not cancellable. Horizon's drag handlers report nothing back, so it fires whether or not one of them used the drag. Fires on the window only, while a screen of the game's or the title's is open (the screens the `_pre` input hooks cover), for real mouse or keyboard input and not a pad's or the keyboard focus's, and only when the matching Pre did not cancel it. It fires on the screen open after the game handled the input, so an input that closed the screen fires nothing.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "ScreenEvent.MouseDragged.Post",
        cancellable = false
    )]
    fn screen_mouse_dragged_post();
    /// The wheel turned over the open screen and the screen's scroll handlers have run (`ScreenEvent.MouseScrolled.Post`, vanilla's `Screen.mouseScrolled`): `scroll` and `modifiers` as in `screen_mouse_scrolled_pre`. Not cancellable. Horizon's scroll handlers report nothing back, so it fires whether or not one of them scrolled. Fires on the window only, while a screen of the game's or the title's is open (the screens the `_pre` input hooks cover), for real mouse or keyboard input and not a pad's or the keyboard focus's, and only when the matching Pre did not cancel it. It fires on the screen open after the game handled the input, so an input that closed the screen fires nothing.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "ScreenEvent.MouseScrolled.Post",
        cancellable = false
    )]
    fn screen_mouse_scrolled_post();
    /// A key press that no screen took (`ScreenEvent.KeyPressed.Post`, vanilla's `Screen.keyPressed` returning false): `key`, `repeat` and `modifiers` as in `screen_key_pressed_pre`. Set `canceled` and the press counts as handled: the game's own key handling (its key bindings, hotbar and toggles) never sees it. It does not fire for a key a screen took (a text box, the focus keys, Escape-close, an inventory key, a merchant or recipe-book key). A key that has no vanilla name fires nothing. The control socket's `key` fires it too, for a press (never a release). Fires on the window only, while a screen of the game's or the title's is open (the screens the `_pre` input hooks cover), for real mouse or keyboard input and not a pad's or the keyboard focus's, and only when the matching Pre did not cancel it. It fires on the screen open after the game handled the input, so an input that closed the screen fires nothing.
    #[hook(
        side = "client",
        contract = "cancel",
        out = "canceled",
        neoforge = "ScreenEvent.KeyPressed.Post",
        cancellable = true
    )]
    fn screen_key_pressed_post();
    /// A key release that no screen took (`ScreenEvent.KeyReleased.Post`, vanilla's `Screen.keyReleased` returning false): `key` and `modifiers` as in `screen_key_released_pre`. Set `canceled` and the release counts as handled: the game's own key handling never sees it, so a held mapping stays down; cancel a release only when you cancelled the press it ends. Fires on the window only, while a screen of the game's or the title's is open (the screens the `_pre` input hooks cover), for real mouse or keyboard input and not a pad's or the keyboard focus's, and only when the matching Pre did not cancel it. It fires on the screen open after the game handled the input, so an input that closed the screen fires nothing.
    #[hook(
        side = "client",
        contract = "cancel",
        out = "canceled",
        neoforge = "ScreenEvent.KeyReleased.Post",
        cancellable = true
    )]
    fn screen_key_released_post();
    /// A typed character that no text box took (`ScreenEvent.CharacterTyped.Post`, vanilla's `Screen.charTyped` returning false), once per printable character that `screen_char_typed_pre` let through: `c` and `modifiers` as there. Set `canceled` and it counts as handled (nothing further reads it, so it changes nothing). It does not fire for text a box took, nor on Options and the title's own screens, where the key's text is routed in place between their boxes. A held edit modifier makes the key a shortcut, so its character is not taken and fires it. Same screens as the key press. Fires on the window only, while a screen of the game's or the title's is open (the screens the `_pre` input hooks cover), for real mouse or keyboard input and not a pad's or the keyboard focus's, and only when the matching Pre did not cancel it. It fires on the screen open after the key acted, as for the Pre.
    #[hook(
        side = "client",
        contract = "cancel",
        out = "canceled",
        neoforge = "ScreenEvent.CharacterTyped.Post",
        cancellable = true
    )]
    fn screen_char_typed_post();
}
