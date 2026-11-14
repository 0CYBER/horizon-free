//! A client mod leaves the world (#3451). The server half is `native_player_disconnect`.
use super::native_entity_refs::tests::with_client_game;
use horizon_client::title::Mode;
use mod_api::{ClientHost, DisconnectError, Text};

fn kicked() -> Text {
    Text::translate("mymod.kicked", vec![])
}

/// The client leaves at the end of the frame, not inside the call, and the disconnect screen
/// shows the reason. With no world joined, the call is refused as unavailable.
#[test]
#[ignore = "daily"]
fn a_client_mod_leaves_at_the_end_of_the_frame_with_its_reason() {
    with_client_game(|game| {
        game.title = None;
        let host: &mut dyn ClientHost = game;
        assert_eq!(
            host.disconnect(Some(kicked())),
            Err(DisconnectError::Unavailable)
        );
        game.native_leave_if_asked().unwrap();
        let (pipe, _server) = horizon_protocol::connection::Pipe::pair();
        game.connect_local(pipe, "Steve").unwrap();
        let host: &mut dyn ClientHost = game;
        assert_eq!(host.disconnect(Some(kicked())), Ok(()));
        assert!(
            game.remote.is_some(),
            "still connected until the frame ends"
        );
        game.native_leave_if_asked().unwrap();
        assert!(game.remote.is_none());
        let mode = &game.title.as_ref().expect("the title screen").mode;
        assert!(
            matches!(
                mode,
                Mode::Multiplayer(horizon_client::multiplayer::Screen::Disconnected { reason, .. })
                    if reason == "mymod.kicked"
            ),
            "the disconnect screen with the reason"
        );
    });
}

/// A request made in one world never leaves the next: the world switches before the frame
/// ends, and the client stays in the new one.
#[test]
#[ignore = "daily"]
fn a_leave_request_does_not_follow_a_world_switch() {
    with_client_game(|game| {
        game.title = None;
        let (pipe, _server) = horizon_protocol::connection::Pipe::pair();
        game.connect_local(pipe, "Steve").unwrap();
        let host: &mut dyn ClientHost = game;
        assert_eq!(host.disconnect(Some(kicked())), Ok(()));
        game.native_lifecycle
            .begin_open(mod_api::Authority::RemoteClient);
        game.native_leave_if_asked().unwrap();
        assert!(game.remote.is_some(), "the new world is not left");
        assert!(game.title.is_none());
        // Nothing is left over for a later frame either.
        game.native_leave_if_asked().unwrap();
        assert!(game.remote.is_some());
    });
}
