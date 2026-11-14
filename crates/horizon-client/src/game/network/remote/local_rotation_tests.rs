//! `set_local_rotation`: a client mod turns the local player's view; the look goes out in the
//! tick's own player sync, one move packet with vanilla's clamp, never a packet of the call's own.
use crate::game::native_client_session::tests::logged_client;
use crate::game::native_entity_refs::tests::with_client_game;
use horizon_client::motion::PlayerMotion;
use horizon_core::geometry::Vec3;
use horizon_protocol::connection::Connection;
use horizon_protocol::packets::play::Serverbound as S;
use mod_api::LocalRotationError;

/// The server's side of the handshake up to play, as `client_targeting`'s test drives it.
fn finish_login(server: &mut Connection) {
    use horizon_protocol::packets::{Empty, configuration as cfg, login};
    server.recv().unwrap(); // Intention.
    server.recv().unwrap(); // Hello.
    server
        .send(
            &login::Clientbound::LoginFinished(login::LoginFinished {
                profile: login::GameProfile {
                    uuid: 42,
                    name: "SessionReadTest".into(),
                    properties: vec![],
                },
                session: 1,
            })
            .encode(),
        )
        .unwrap();
    server.recv().unwrap(); // Login acknowledged.
    server.recv().unwrap(); // Brand.
    server.recv().unwrap(); // Client information.
    server
        .send(&cfg::Clientbound::FinishConfiguration(Empty).encode())
        .unwrap();
    server.recv().unwrap(); // Finish configuration.
}

/// The move packets the client sent before a fence, as (kind, yaw, pitch).
fn sent_moves(
    game: &crate::game::Game<'_>,
    server: &mut Connection,
) -> Vec<(&'static str, f32, f32)> {
    game.send(S::Punch(horizon_protocol::packets::Empty));
    let mut moves = Vec::new();
    loop {
        let (id, payload) = server.recv().unwrap();
        match S::decode(id, &payload).unwrap() {
            S::MovePlayerPosRot(p) => moves.push(("pos_rot", p.yaw, p.pitch)),
            S::MovePlayerRot(p) => moves.push(("rot", p.yaw, p.pitch)),
            S::MovePlayerPos(_) => moves.push(("pos", f32::NAN, f32::NAN)),
            S::MovePlayerStatus(_) => moves.push(("status", f32::NAN, f32::NAN)),
            S::Punch(_) => return moves,
            _ => {}
        }
    }
}

#[test]
#[ignore = "daily"]
fn a_mod_set_look_is_clamped_and_sent_once_by_the_tick() {
    mod_platform::testing::with_mods(Vec::new().into(), || {
        with_client_game(|game| {
            assert_eq!(
                game.set_local_rotation(10.0, 5.0),
                Err(LocalRotationError::NoWorld)
            );
            let mut server = Connection::local(logged_client(game));
            server
                .reader
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            finish_login(&mut server);
            game.title = None;
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
            // The baseline: the first sync sends the position.
            game.remote_send_state(0);
            sent_moves(game, &mut server);

            // Yaw stays unwrapped, pitch is clamped; the call itself sends nothing.
            assert_eq!(game.set_local_rotation(450.0, 120.0), Ok(()));
            assert_eq!((game.yaw, game.pitch), (450.0, 90.0));
            assert!(sent_moves(game, &mut server).is_empty());
            for (yaw, pitch) in [
                (f32::NAN, 0.0),
                (0.0, f32::INFINITY),
                (f32::NEG_INFINITY, 0.0),
            ] {
                assert_eq!(
                    game.set_local_rotation(yaw, pitch),
                    Err(LocalRotationError::NotFinite)
                );
                assert_eq!((game.yaw, game.pitch), (450.0, 90.0), "unchanged");
            }
            game.remote_send_state(0);
            assert_eq!(sent_moves(game, &mut server), [("rot", 450.0, 90.0)]);

            // Two calls in one tick: one packet, the last values.
            game.set_local_rotation(30.0, -10.0).unwrap();
            game.set_local_rotation(-200.0, 45.0).unwrap();
            game.remote_send_state(0);
            assert_eq!(sent_moves(game, &mut server), [("rot", -200.0, 45.0)]);
            game.remote_send_state(0);
            assert!(sent_moves(game, &mut server).is_empty(), "nothing new");

            // Riding: the passenger's rotation-only packet.
            game.riding = Some(99);
            game.set_local_rotation(12.0, 3.0).unwrap();
            game.remote_send_state(0);
            assert_eq!(sent_moves(game, &mut server), [("rot", 12.0, 3.0)]);
            game.riding = None;

            game.remote.as_mut().unwrap().logged_in = false;
            assert_eq!(
                game.set_local_rotation(0.0, 0.0),
                Err(LocalRotationError::NoWorld)
            );
        });
    });
}
