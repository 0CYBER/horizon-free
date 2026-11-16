//! Chat screen helper carried from upstream.
pub trait Notes {
    #[notes = r#"`ClientHost::open_chat(text)` opens the chat screen with `text` typed in for the player to finish and send, as T and / open it; nothing is sent; an error with no player in a world or while another screen is open. Bundles need "hooks": @API:open_chat@."#]
    fn open_chat();
}
