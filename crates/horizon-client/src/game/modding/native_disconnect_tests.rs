//! A client mod leaves the world (arismoko/horizon#3451). The server half is `native_player_disconnect`.
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
                    if reason.plain() == "mymod.kicked"
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

/// The disconnect screen renders the reason in the player's language: a translatable reason
/// reads as its translation and a plain-text one is unchanged. Both cross the same fallible
/// text conversion the host call uses.
#[test]
#[ignore = "daily"]
fn a_translatable_reason_reads_in_the_players_language() {
    let language = horizon_client::lang::Language::of(
        "fr_fr",
        [
            ("disconnect.lost".into(), "Connexion perdue".into()),
            ("mymod.kicked".into(), "Expulsé par le mod".into()),
        ],
    );
    let ((), _missed) = horizon_client::lang::scoped(language, || {
        for (sent, expected) in [
            (kicked(), "Expulsé par le mod"),
            (Text::literal("You were kicked"), "You were kicked"),
        ] {
            let mut seen = None;
            // A fresh game per case: the first leave saves the integrated world, and a
            // request made while that save is pending is refused as unavailable.
            with_client_game(|game| {
                game.title = None;
                let (pipe, _server) = horizon_protocol::connection::Pipe::pair();
                game.connect_local(pipe, "Steve").unwrap();
                let host: &mut dyn ClientHost = game;
                assert_eq!(host.disconnect(Some(sent)), Ok(()));
                game.native_leave_if_asked().unwrap();
                seen = game.title.as_ref().and_then(|title| match &title.mode {
                    Mode::Multiplayer(horizon_client::multiplayer::Screen::Disconnected {
                        reason,
                        ..
                    }) => Some(reason.plain()),
                    _ => None,
                });
            });
            assert_eq!(
                seen.as_deref(),
                Some(expected),
                "the disconnect screen reads the reason in the player's language"
            );
        }
    });
}

/// A malformed reason is refused by name through the generated forwarder, before anything
/// changes: the client stays connected and no screen is built.
#[test]
#[ignore = "daily"]
fn a_too_deep_reason_is_refused_by_name_and_leaves_the_world_joined() {
    with_client_game(|game| {
        game.title = None;
        let (pipe, _server) = horizon_protocol::connection::Pipe::pair();
        game.connect_local(pipe, "Steve").unwrap();
        let mut too_deep = Text::literal("leaf");
        for _ in 1..513 {
            too_deep = Text::literal("parent").append(too_deep);
        }
        let host: &mut dyn ClientHost = game;
        assert_eq!(
            host.disconnect(Some(too_deep)),
            Err(DisconnectError::Text(mod_api::text::TextFormError::TooDeep))
        );
        assert!(game.remote.is_some(), "the refused call leaves the world");
        game.native_leave_if_asked().unwrap();
        assert!(game.remote.is_some());
        assert!(game.title.is_none());
    });
}
