//! `ServerGamePacketListenerImpl.handleSignUpdate`: a text the chat filter refuses closes the
//! editor of the player who sent it, never the editor of the player who is writing on the sign.
use crate::game::native_guest_turn_tests::with_guests;
use horizon_client::{containers::BlockEntity, signs::SignData};
use horizon_protocol::packets::play::SignUpdate;
use horizon_server_files::chat_guard::{Action, ChatGuard, Mute, Place};

/// A filter that refuses any text holding "bad".
struct Refuses;

impl ChatGuard for Refuses {
    fn matches(&self, text: &str) -> bool {
        text.contains("bad")
    }
    fn muted(&self, _: u128, _: i64) -> Option<&Mute> {
        None
    }
    fn mutes(&self, _: i64) -> Vec<&Mute> {
        Vec::new()
    }
    fn mute(&mut self, _: u128, _: &str, _: Option<i64>, _: &str, _: Option<String>) {}
    fn strike(&mut self, _: u128, _: &str, _: i64) -> (u32, Action) {
        (1, Action::Refused)
    }
    fn unmute(&mut self, _: &str) -> Option<String> {
        None
    }
    fn log(
        &mut self,
        _: i64,
        _: &str,
        _: u128,
        _: Place,
        _: u32,
        _: Action,
        _: &str,
    ) -> Result<(), String> {
        Ok(())
    }
}

fn update(text: &str) -> SignUpdate {
    SignUpdate {
        pos: [2, 4, 3],
        lines: [text.into(), String::new(), String::new(), String::new()],
        front: true,
    }
}

fn editor(game: &crate::game::Game<'_>) -> Option<u128> {
    match game.world.block_entities.get(&(2, 4, 3)) {
        Some(BlockEntity::Sign(sign)) => sign.allowed_editor,
        _ => panic!("the sign is gone"),
    }
}

/// Guest 13 sends refused text for the sign guest 12 is editing: 12 still may write on it. The
/// text 12 sends then goes through, and 13's own sign is left as it was.
#[test]
#[ignore = "daily"]
fn a_refused_text_from_another_player_keeps_the_editor() {
    with_guests(|game| {
        game.lan
            .as_mut()
            .unwrap()
            .hub
            .set_chat_filter(Some(Box::new(Refuses)));
        game.world.block_entities.insert(
            (2, 4, 3),
            BlockEntity::Sign(SignData {
                allowed_editor: Some(12),
                ..SignData::default()
            }),
        );

        game.lan_sign_update(13, update("bad words"));
        assert_eq!(editor(game), Some(12), "the editor of 12 was closed by 13");

        // The editor's own refused text still closes it.
        game.lan_sign_update(12, update("bad words"));
        assert_eq!(editor(game), None);
    });
}
