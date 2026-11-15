pub trait Hooks {
    /// `chat_suggestion_applied(original, start, end, suggestion, input)` reports a chat completion after Tab or a click. The range is in UTF-16 units of the list's original input, including when cycling; input is the resulting length-limited field value. Client-only, read-only. Bundles need "hooks": @API:chat_suggestion_applied@.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "none (vanilla command suggestions; NeoForge adds no event)"
    )]
    fn chat_suggestion_applied();
}
