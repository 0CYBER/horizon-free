# Completing chat text

A client mod offers its own completions for what the player types in the chat box, beside the
player names vanilla offers there (`ClientSuggestionProvider.getCustomTabSuggestions`): emoji
codes, channel names, stock phrases. They show in vanilla's suggestion list when the player presses
Tab, and Tab or a click puts one in the box. Commands keep their own completions
(`register_client_commands`, the server's tree); this hook is for chat text alone.

```rust,ignore
use mod_api::*;

struct Emoji;

const CODES: &[(&str, &str)] = &[(":heart:", "❤"), (":smile:", "😄"), (":star:", "⭐")];

#[mod_api::horizon_mod]
impl Hooks for Emoji {
    fn chat_suggestions(&self, _host: &dyn ClientHost, e: &mut ChatSuggestionsEvent<'_>) {
        // The word the completion replaces: from the last space before the cursor.
        let before: String = e.input.chars().take(e.cursor).collect();
        let word = &before[before.rfind(' ').map_or(0, |space| space + 1)..];
        if word.starts_with(':') {
            for (code, emoji) in CODES {
                if code.starts_with(word) {
                    e.suggestions.push((*emoji).to_owned());
                }
            }
        }
    }
}
```

## When it is asked

- `chat_suggestions` is asked once each time the chat box works out its list for text that doesn't
  start with `/`: the text changed, or the chat opened or resized holding it. It is asked before
  the next key reaches the box, so a Tab right after typing shows your entries, and by the next frame
  for a change no key follows (a click, a paste).
- It is never asked for a command or for blank text.
- `input` is the whole text in the box and `cursor` the cursor, in Unicode scalar characters.

## How the list shows them

- Each entry replaces the word before the cursor: the text from the last space up to the cursor,
  as vanilla's player names do. The text after the cursor stays.
- The game doesn't filter your entries against that word, so push only what fits it. An entry that
  only repeats the word is left out.
- Your entries join vanilla's list as two Brigadier lists merge: one of each, sorted ignoring case.
  Tab then shows the entries that start with the typed word first, as vanilla does.
- Picking one fires `chat_suggestion_applied` with the entry and the replaced range.
