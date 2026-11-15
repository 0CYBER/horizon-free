//! The `chat_recipient` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A public chat line from `sender` (`sender_name`) is about to reach the placed player
    /// `recipient`: a player's chat after `server_chat` let it through, or a `/say` or `/me`
    /// (signed or not). `message` is the text players see: what `server_chat` left, or the
    /// command's text. Set `hidden` and that one receiver is not sent the line at all, no chat
    /// index and no pending acknowledgement; the others still get it, signed as it was sent, so
    /// its signature still verifies for them. It is asked once for each placed receiver, the
    /// sender included, before any of them is sent the line, in the order the hub lists them.
    /// Whispers (`/msg`) are not public and are not heard here, and neither is the line the
    /// server's log keeps: hiding is per receiver. This is not a second `server_chat`: that
    /// hook is heard once for a line, only for typed chat, and cancels it for everyone; this one
    /// is per receiver, and covers `/say` and `/me`. With no listener nothing is asked and
    /// nothing is allocated. A fault or an epoch timeout in a wasm listener answers nothing, so
    /// the line is sent, like every cancellable hook. No player's state is swapped in.
    /// Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "hidden",
        neoforge = "none (NeoForge sends a chat line to every player; it has no per-receiver event)",
        cancellable = true
    )]
    fn chat_recipient();
}
