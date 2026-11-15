//! The `client_chat_received` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `ClientChatReceivedEvent`: a chat line from the server is
    /// about to show, once Chat Delay lets it through. `system` is a system
    /// message (`ClientChatReceivedEvent.System`), shown in the action bar
    /// when `overlay` is set; otherwise a player or disguised message in its
    /// chat type `chat_type` (`getBoundChatType`, its registry name; `None`
    /// for a system message or an inline chat type). `sender` is the player's
    /// UUID, 0 (`Util.NIL_UUID`) for system and disguised messages.
    /// `player_message` is the message as the player signed it
    /// (`ClientChatReceivedEvent.Player.getPlayerChatMessage`): its body,
    /// whether a signature validated, timestamp, salt, the server's unsigned
    /// replacement and the filter mask; `None` for system and disguised
    /// messages. `message`
    /// starts as the decorated line; replace it and the chat shows the new
    /// one, while the narrator still says the original. Set `cancel` and it
    /// doesn't show: a delayed message lets the next one through. Lines the
    /// client adds itself and validation errors are not told. Client-side.
    #[hook(
        side = "client",
        contract = "cancel",
        out = "cancel",
        version = 2,
        neoforge = "ClientChatReceivedEvent, ClientChatReceivedEvent.Player, ClientChatReceivedEvent.System",
        cancellable = true
    )]
    fn client_chat_received(
        system: bool,
        overlay: bool,
        sender: u128,
        chat_type: Option<&str>,
        player_message: Option<&PlayerChatInfo>,
        message: &mut Text,
        cancel: &mut bool,
    );
}
