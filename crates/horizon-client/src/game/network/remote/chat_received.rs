//! `ClientChatReceivedEvent` for the lines the server sends: the queued player
//! message and the hook that may replace or cancel a line.
use super::*;

/// A player message Chat Delay holds back, its signature and trust level,
/// what the narrator says of it once it shows, and who sent it in which
/// chat type, for mods told when it shows (`ClientChatReceivedEvent`).
pub(super) struct QueuedChat {
    pub(super) line: horizon_client::text::Text,
    pub(super) signature: Option<Signature>,
    pub(super) trust: ChatTrust,
    pub(super) narration: String,
    /// The player's UUID, 0 for disguised chat (`Util.NIL_UUID`).
    pub(super) sender: u128,
    /// The chat type's network ID (`ChatType.Bound.chatType`).
    pub(super) chat_type: i32,
    /// The message as the player signed it, read only while a mod listens;
    /// `None` for disguised chat and validation errors.
    pub(super) player: Option<mod_api::PlayerChatInfo>,
}

/// `PlayerChatMessage` of a `ClientboundPlayerChatPacket`, for
/// `ClientChatReceivedEvent.Player`; `signed` when its signature validated.
pub(super) fn player_chat_info(p: &play::PlayerChat, signed: bool) -> mod_api::PlayerChatInfo {
    use horizon_client::scoreboard::{comp_text, tag_to_json};
    let filter = match p.filter.0 {
        1 => mod_api::ChatFilter::FullyFiltered,
        2 => mod_api::ChatFilter::Partial(p.filter.1.iter().map(|&w| w as u64).collect()),
        _ => mod_api::ChatFilter::PassThrough,
    };
    mod_api::PlayerChatInfo {
        body: p.content.clone(),
        signed,
        timestamp_ms: p.timestamp,
        salt: p.salt,
        // A line the API's text form cannot hold is left out.
        unsigned_content: p
            .unsigned_content
            .as_ref()
            .and_then(|t| horizon_client_lib::api_text::to_api(&comp_text(&tag_to_json(t))).ok()),
        filter,
    }
}

impl Game<'_> {
    /// NeoForge's `ClientChatReceivedEvent` (`ClientHooks.onClientChat`,
    /// `onClientPlayerChat` and `onClientSystemChat`): mods may replace
    /// `line` or cancel it. False when cancelled.
    pub(super) fn chat_received(
        &mut self,
        system: bool,
        overlay: bool,
        sender: u128,
        chat_type: Option<i32>,
        player: Option<&mod_api::PlayerChatInfo>,
        line: &mut horizon_client::text::Text,
    ) -> bool {
        if !mod_api::__private::listened(mod_api::HookId::ClientChatReceived) {
            return true;
        }
        let registries = self.remote.as_ref().map(|r| Arc::clone(&r.registries));
        let name = chat_type
            .zip(registries.as_deref())
            .and_then(|(id, registries)| registries.name("minecraft:chat_type", id));
        // A line the API's text form cannot hold is not told, as before.
        let Ok(mut message) = horizon_client_lib::api_text::to_api(line) else {
            return true;
        };
        let mut cancel = false;
        self.native_client_chat_received(mod_api::__private::ClientChatReceivedCall {
            system,
            overlay,
            sender,
            chat_type: name,
            player_message: player,
            message: &mut message,
            cancel: &mut cancel,
        });
        if let Ok(text) = horizon_client_lib::api_text::to_engine(&message) {
            *line = text;
        }
        !cancel
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The packet to `PlayerChatInfo` conversion alone (the fake signature is
    /// never validated; `signed` is passed in): the body is what the player
    /// typed, the unsigned content the server's line, and a partial filter
    /// keeps its bitset words.
    #[test]
    fn a_signed_relayed_message_keeps_its_body_and_the_servers_line() {
        let p = play::PlayerChat {
            global_index: 0,
            sender: 7,
            index: 0,
            signature: Some(vec![1; 256]),
            content: "hello".into(),
            timestamp: 1_700_000_000_000,
            salt: -5,
            last_seen: Vec::new(),
            unsigned_content: Some(Tag::String("[mod] hello".into())),
            filter: (2, vec![-1, 3]),
            chat_type: play::ChatTypeBound {
                chat_type: 0,
                name: Tag::String("Steve".into()),
                target: None,
            },
        };
        let info = player_chat_info(&p, true);
        assert!(info.signed);
        assert_eq!(info.body, "hello");
        assert_eq!((info.timestamp_ms, info.salt), (1_700_000_000_000, -5));
        let unsigned = info.unsigned_content.expect("the server's line");
        let mod_api::text::TextContent::Literal(text) = &unsigned.content else {
            panic!("expected the server's literal line, got {unsigned:?}");
        };
        assert_eq!(text, "[mod] hello");
        assert_eq!(info.filter, mod_api::ChatFilter::Partial(vec![u64::MAX, 3]));
        let passed = play::PlayerChat {
            filter: (0, Vec::new()),
            ..p
        };
        assert_eq!(
            player_chat_info(&passed, false).filter,
            mod_api::ChatFilter::PassThrough
        );
    }
}
