//! The `screen_char_typed_pre` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0109

pub trait Hooks {
    /// A character was typed at the open screen
    /// (`ScreenEvent.CharacterTyped.Pre`, vanilla's `Screen.charTyped`):
    /// `screen` is the screen it is, `c` the character, `modifiers` GLFW's
    /// bits (shift 1, control 2, alt 4, super 8), and `field` a copy of the
    /// focused text box. Set `canceled` and that one character never reaches
    /// a text box.
    ///
    /// `field` is `None` with no box focused, and for the boxes that keep
    /// their own validators: Options' and World Options' fields, the game
    /// rules' and the block editor's. `cursor` and `highlight` count Unicode
    /// scalar characters, and a character sees the box as the one before it
    /// left it. Change `value`, `cursor` or `highlight` and the box takes the
    /// change, cancelled or not, so cancel to replace the character yourself.
    /// New text goes through the box's own filter and length limit, and what
    /// follows the box (chat suggestions, a search's results, the anvil's
    /// name) follows it. `max_length`, `multiline` and whether `field` is
    /// `None` stay the box's: a change to them does nothing.
    ///
    /// It fires once per printable character of a key press's text and once
    /// per character of the input method's committed text, never for a control
    /// character (Enter, Backspace, Tab, Delete, Escape, a Control letter):
    /// those are keys, and the game reads them as keys, not as text. A key's
    /// characters fire after the key has acted, as GLFW's char callback comes
    /// after its key callback (`KeyboardHandler.charTyped`): `screen` is the
    /// screen open then, so a key that closes the screen (the inventory key)
    /// leaves none to hear its character, where vanilla's closed screen hears
    /// nothing, and one that moves the focus types into the box it moved to.
    /// They fire even when `screen_key_pressed_pre` cancelled the press
    /// that carried the text, as NeoForge delivers `CharacterTyped.Pre` for a
    /// cancelled `KeyPressed.Pre`: the characters left reach the text box
    /// without the key's own effects. Cancelling one character drops only that
    /// one. The control socket's `type` fires it too, but not for its `\b`
    /// backspace. Same screens as the key press, and it fires with no text box
    /// focused (there is nothing to take the character then).
    #[hook(
        side = "client",
        contract = "cancel",
        out = "canceled",
        neoforge = "ScreenEvent.CharacterTyped.Pre",
        cancellable = true
    )]
    fn screen_char_typed_pre();
}
