/// What of a key press's text is characters for the
/// `ScreenEvent.CharacterTyped.Pre` hook: control ones are gone (the key's own
/// handling takes those, and vanilla fires no `charTyped` for them).
/// With the edit modifier held the key is a shortcut and its characters are
/// not text (`types_text`): no hook hears them, as vanilla's `textInput`
/// gets none for Ctrl+A.
pub(super) fn chars_to_type(text: &str, types_text: bool) -> String {
    if !types_text {
        return String::new();
    }
    text.chars().filter(|c| !c.is_control()).collect()
}
