//! The `server_chat` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `ServerChatEvent`: a player's chat message, through their
    /// chain and the chat filter, is about to be broadcast.
    /// `player_id` is the server's entity id for the player, `player` their
    /// UUID, `username` their name and `raw_text` what they typed. `message`
    /// starts as that text; replace it and players see the new message
    /// (`withUnsignedContent`: a signed message keeps its signature), and
    /// the log shows it. Set `cancel` and nobody sees it, and it doesn't
    /// count toward the chat spam limit. Commands (`/say`, `/msg`...) are
    /// not chat here. Server-side, with no player's state swapped in.
    /// A component deeper than 512 nodes refuses capture or the whole writeback;
    /// no event field is applied if any Text field fails conversion.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "ServerChatEvent",
        cancellable = true
    )]
    fn server_chat(
        player_id: i32,
        player: u128,
        username: &str,
        raw_text: &str,
        message: &mut Text,
        replies: &mut Vec<Text>,
        cancel: &mut bool,
    );
}
