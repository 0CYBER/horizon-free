//! The `chat_suggestions` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Completions for plain chat text, beside the player names vanilla offers there
    /// (`ClientSuggestionProvider.getCustomTabSuggestions`). Asked once each time the chat box's
    /// list is worked out for text that doesn't start with `/` (the text changed, or the chat
    /// opened or resized holding it), before the next key reaches the box or the next frame is
    /// drawn; never for a command or for blank text. `input` is the whole text and `cursor` the
    /// cursor, in Unicode scalar characters. Push the completions to `suggestions`: each
    /// replaces the word before the cursor, as a player name does. The game does not filter
    /// them against that word, so push only what fits it. They join vanilla's list, which Tab
    /// shows as vanilla's: one of each, sorted ignoring case, those starting with the typed word
    /// first; one that only repeats the word is left out. Picking one fires
    /// `chat_suggestion_applied`. Client-side.
    #[hook(
        side = "client",
        contract = "collect",
        out = "suggestions",
        cancellable = false,
        neoforge = "none (vanilla's ClientSuggestionProvider.getCustomTabSuggestions; NeoForge adds no event)"
    )]
    fn chat_suggestions();
}
