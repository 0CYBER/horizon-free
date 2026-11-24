use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// Completions for plain chat text `input` (no `/`) with the cursor at
    /// `cursor`, in characters, beside the player names vanilla offers: push
    /// them to `suggestions`. Each replaces the word before the cursor and
    /// joins vanilla's list, unfiltered. Asked once each time the chat box's
    /// list is worked out for such text. Client-side.
    pub(in crate::game) fn native_chat_suggestions(
        &self,
        input: &str,
        cursor: usize,
        suggestions: &mut Vec<String>,
    ) {
        self.fire_chat_suggestions(input, cursor, suggestions);
    }
}
