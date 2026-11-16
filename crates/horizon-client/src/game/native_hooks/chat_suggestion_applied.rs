use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// A chat completion applied after Tab or a click. `start..end` is in
    /// UTF-16 units of the list's `original` input, including when cycling;
    /// `input` is the resulting length-limited field value. Client-side,
    /// read-only.
    pub(in crate::game) fn native_chat_suggestion_applied(
        &self,
        original: &str,
        start: usize,
        end: usize,
        suggestion: &str,
        input: &str,
    ) {
        self.fire_chat_suggestion_applied(original, start, end, suggestion, input);
    }
}
