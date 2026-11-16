//! Remote saves are no-ops, even while another pass owns a visibility lease.
use super::*;

#[test]
#[ignore = "daily"]
fn remote_save_with_a_held_lease_is_still_a_silent_noop() {
    native_entity_refs::tests::with_client_game(|game| {
        let (pipe, _server) = horizon_protocol::connection::Pipe::pair();
        game.connect_local(pipe, "RemoteSave").unwrap();
        assert!(game.remote.is_some());
        let _lease = horizon_client::join_seq::visibility::PassLease::enter();
        let message = game.message.clone();
        assert!(game.save_game().is_ok());
        assert!(game.save_game_with(false).is_ok());
        assert_eq!(
            game.message, message,
            "a no-op save must not display a refusal"
        );
    });
}
