//! A resource pack a mod offers one player, and why an offer or a removal is refused.
use crate::Text;

/// A resource pack to offer one player's client (`ClientboundResourcePackPushPacket`).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct ResourcePackOffer {
    /// The pack's identity on the client: pushing the same id again replaces it, and
    /// `pop_resource_pack` and `resource_pack_status_v2` name it.
    pub id: u128,
    /// Where the client downloads the zip from; at most 32767 UTF-16 code units.
    pub url: String,
    /// The zip's SHA-1 as 40 hexadecimal characters, for the client to check; empty for no check.
    /// The protocol caps this string at 40 UTF-16 code units.
    pub sha1: String,
    /// The client's screen offers no way to keep playing without it.
    pub required: bool,
    /// The line the client's prompt shows with the pack.
    pub prompt: Option<Text>,
}

impl ResourcePackOffer {
    /// A pack that is optional, with no check and no prompt.
    pub fn new(id: u128, url: impl Into<String>) -> Self {
        Self {
            id,
            url: url.into(),
            sha1: String::new(),
            required: false,
            prompt: None,
        }
    }
}

/// Why `player.push_resource_pack` or `player.pop_resource_pack` sent nothing.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum PlayerPackRefusal {
    /// The call was made on a client; these are the server's.
    Client,
    /// The player handle no longer identifies a player in the server's list.
    Stale,
    /// The player has no connection to send to (a fake player).
    NoConnection,
    /// The URL is over 32767 UTF-16 code units, the most a protocol string holds.
    UrlTooLong,
    /// The SHA-1 is over 40 UTF-16 code units (`ClientboundResourcePackPushPacket` refuses it).
    HashTooLong,
    /// The prompt is text the boundary refuses (too large or too deeply nested).
    BadPrompt(crate::TextFormError),
}

impl crate::NoBackend for PlayerPackRefusal {
    fn no_backend() -> Self {
        Self::Stale
    }
}
