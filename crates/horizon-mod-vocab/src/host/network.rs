//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The client host's methods of the `network` area, implemented by the game and reached through [`crate::ClientHost`].
#[horizon_mod_api_macros::host_area]
pub trait NetworkClientHost {
    /// Side: client.
    /// Opens the chat screen with `text` typed in for the player to finish and
    /// send, as T and / open it; nothing is sent. Returns an error with no
    /// player in a world or while another screen is open.
    #[access(WorldOnly)]
    fn open_chat(&mut self, text: &str) -> Result<(), String>;
    /// Side: client.
    /// Adds `line` to this client's chat HUD and history (vanilla's
    /// `ChatComponent.addClientSystemMessage`): it shows here only, so
    /// nothing is sent to the server. The Chat option's server-message filter
    /// doesn't hide it (`GuiMessageSource.SYSTEM_CLIENT`), and
    /// `client_chat_received` is not told: vanilla fires that event from
    /// `ChatListener` for lines the server sent, not for lines the client adds
    /// itself. Returns an error if the text exceeds the boundary's size or
    /// nesting limits; rejected text is not added.
    #[access(WorldOnly)]
    fn chat_line(&mut self, line: Text) -> Result<(), TextFormError>;
    /// Side: client.
    /// A mod's chat line or command (`/` first), sent as `send_chat` sends
    /// it (ClientPacketListener.sendChat / sendCommand), to any server: no
    /// chat history, and no client chat event (vanilla's `ClientChatEvent`
    /// comes only from ChatScreen). An error when the player isn't in a
    /// server's play state or the Chat option blocks the message. API
    /// 404.
    #[access(WorldOnly)]
    fn send_chat(&mut self, message: &str) -> Result<(), String>;
    /// Side: client.
    /// Submits a vanilla play packet to any server, including unrelated plugin
    /// payloads. The body is validated once and outgoing credit is reserved before
    /// acceptance. Publication waits for the current root transaction to commit;
    /// rollback, disconnect, or a protocol incarnation change cancels it.
    /// Registered and engine-reserved channels are refused: use their payload or
    /// service API. Unrelated plugin bodies retain vanilla's discard-codec limit.
    /// No effects are applied locally. Sequence numbers, signatures, and client
    /// state required by a packet remain the mod's responsibility. API 409.
    #[access(WorldOnly)]
    fn send_packet(&mut self, packet: ServerboundPlayPacket) -> Result<(), SendPacketError>;
    /// Side: client.
    /// Leaves the server or the single-player world (NeoForge's
    /// `IClientCommonPacketListenerExtension.disconnect(Component)`), at the end of this frame,
    /// not inside the calling hook. With a reason, the disconnect screen shows it (a
    /// single-player world is saved first); with none, it leaves as the pause menu's Disconnect
    /// does. The request belongs to the world it was made in: one made while that world closes
    /// is dropped, never carried into the next. A second call in the same frame replaces the
    /// reason. `DisconnectError::Unavailable` when no server or world is joined.
    #[access(WorldOnly)]
    /// Refuses text deeper than 512 component nodes before applying any changes.
    fn disconnect(&mut self, reason: Option<Text>) -> Result<(), DisconnectError>;
}
