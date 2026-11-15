//! A window crossing a passage its server told it of (arismoko/horizon#3775): it moves itself through,
//! tells the server, and the server's all-relative zero position packet leaves it where it
//! is with its velocity, and stops its mining.
use super::tests::connect_menu_test;
use crate::game::native_entity_refs::tests::with_client_game;
use horizon_client::chunk::Chunk;
use horizon_client::lan::hub::passage_cross::refusal_code;
use horizon_core::geometry::Vec3;
use horizon_protocol::connection::{ConnReader, ConnWriter};
use horizon_protocol::packets::play::{self, Clientbound as C, Serverbound as S, relative};
use horizon_protocol::passage_sync::{Cross, Message, PassageForm, decode_cross, encode};
use mod_api::{PassageCrossError, PassageCrossRefusal, PassageId};
use std::sync::Arc;

/// What the window sent since the last call, as play packets.
fn sent(reader: &mut ConnReader) -> Vec<S> {
    reader
        .set_read_timeout(Some(std::time::Duration::from_millis(200)))
        .unwrap();
    let mut packets = Vec::new();
    while let Ok((id, body)) = reader.recv() {
        packets.extend(S::decode(id, &body).ok());
    }
    packets
}

/// A window logged in to the overworld, its player at `at` facing south, holding a 2 by 3
/// doorway (passage 1) at (0.5, 65.5, 8.5) facing south, which comes out at (24.5, 65.5,
/// 8.5) turned a clockwise quarter.
fn scene(game: &mut crate::game::Game<'_>, at: [f64; 3]) -> (ConnReader, ConnWriter) {
    let (mut reader, writer) = connect_menu_test(game);
    game.handle_packet(C::Login(play::Login {
        player_id: 7,
        hardcore: false,
        levels: vec!["minecraft:overworld".into()],
        max_players: 20,
        chunk_radius: 8,
        simulation_distance: 8,
        reduced_debug_info: false,
        show_death_screen: true,
        limited_crafting: false,
        spawn: play::SpawnInfo {
            dimension_type: 0,
            dimension: "minecraft:overworld".into(),
            seed: 1,
            game_type: 0,
            previous_game_type: None,
            debug: false,
            flat: false,
            last_death: None,
            portal_cooldown: 0,
            sea_level: 63,
        },
        online_mode: false,
        enforces_secure_chat: false,
    }))
    .unwrap();
    game.handle_packet(C::PlayerPosition(play::PlayerPosition {
        teleport: 1,
        position: at,
        delta: [0.0; 3],
        yaw: 0.0,
        pitch: 0.0,
        relatives: 0,
    }))
    .unwrap();
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    let doorway = PassageForm {
        id: 1,
        origin: [0.5, 65.5, 8.5],
        axis_w: [1.0, 0.0, 0.0],
        axis_h: [0.0, 1.0, 0.0],
        width: 2.0,
        height: 3.0,
        destination: [24.5, 65.5, 8.5],
        turn: 1,
    };
    assert!(
        game.remote_passage_sync(&encode(&Message::Add(vec![doorway])).unwrap())
            .is_empty()
    );
    sent(&mut reader);
    (reader, writer)
}

/// The player walks south into the doorway while digging: it comes out 24 blocks east,
/// facing and moving west, its last tick's place carried with it, and the server is told
/// the passage and the feet before. The server's acceptance, all relative and zero, leaves
/// position and velocity as they are, is confirmed with where the player is, aborts the
/// dig, starts the attack strength again and marks the block predictions made so far.
#[test]
#[ignore = "daily"]
fn a_window_crosses_and_keeps_its_velocity_through_the_acceptance() {
    with_client_game(|game| {
        let (mut reader, _server) = scene(game, [0.5, 65.0, 8.25]);
        let player = game.player.as_mut().unwrap();
        player.velocity = Vec3::new(0.0, -0.0784, 0.25);
        game.previous_position = Vec3::new(0.5, 65.0, 8.0);

        assert_eq!(game.cross_passage(PassageId::from_raw(1)), Ok(()));
        let player = game.player.as_ref().unwrap();
        assert_eq!(player.position, Vec3::new(24.75, 65.0, 8.5));
        assert_eq!(player.velocity, Vec3::new(-0.25, -0.0784, 0.0));
        assert_eq!(game.previous_position, Vec3::new(25.0, 65.0, 8.5));
        assert_eq!(game.yaw, 90.0);
        let crossed: Vec<_> = sent(&mut reader)
            .into_iter()
            .filter_map(|p| match p {
                S::CustomPayload(p) if p.channel == "horizon:passage_cross" => {
                    Some(decode_cross(&p.data).unwrap())
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            crossed,
            [Cross {
                id: 1,
                from: [0.5, 65.0, 8.25]
            }]
        );
        assert_eq!(
            game.cross_passage(PassageId::from_raw(1)),
            Err(PassageCrossError::Pending)
        );

        // Digging a block, with predictions up to sequence 7, when the acceptance comes.
        let r = game.remote.as_mut().unwrap();
        r.is_destroying = true;
        r.destroying = Some(([3, 64, 3], 1, 0.5));
        r.sequence = 7;
        game.attack_ticks = 15;
        let all = relative::X
            | relative::Y
            | relative::Z
            | relative::Y_ROT
            | relative::X_ROT
            | relative::DELTA_X
            | relative::DELTA_Y
            | relative::DELTA_Z
            | relative::ROTATE_DELTA;
        game.handle_packet(C::PlayerPosition(play::PlayerPosition {
            teleport: 2,
            position: [0.0; 3],
            delta: [0.0; 3],
            yaw: 0.0,
            pitch: 0.0,
            relatives: all,
        }))
        .unwrap();
        let player = game.player.as_ref().unwrap();
        assert_eq!(player.position, Vec3::new(24.75, 65.0, 8.5));
        assert_eq!(player.velocity, Vec3::new(-0.25, -0.0784, 0.0));
        assert_eq!((game.yaw, game.pitch), (90.0, 0.0));
        let r = game.remote.as_ref().unwrap();
        assert!(!r.is_destroying);
        assert_eq!(r.destroying, Some(([3, 64, 3], 1, 0.0)));
        assert_eq!(r.last_teleport_sequence, 7);
        assert_eq!(game.attack_ticks, 0);
        let answers = sent(&mut reader);
        assert!(answers.iter().any(|p| matches!(
            p,
            S::AcceptTeleportation(a) if a.id == 2 && [a.x, a.y, a.z] == [24.75, 65.0, 8.5]
        )));
        assert!(answers.iter().any(|p| matches!(
            p,
            S::PlayerAction(a) if a.action == 2 && a.pos == [3, 64, 3]
        )));
        // Answered, the next crossing is the window's to try: this one is too far now.
        assert_eq!(
            game.cross_passage(PassageId::from_raw(1)),
            Err(PassageCrossError::TooFar { limit: 20 })
        );
    });
}

/// Refused on the window with nothing moved or sent: a passage it does not hold, and while
/// riding. The server's refusal is kept for the mods until the next crossing.
#[test]
#[ignore = "daily"]
fn a_window_refuses_what_it_cannot_cross_and_keeps_the_servers_refusal() {
    with_client_game(|game| {
        let (mut reader, _server) = scene(game, [0.5, 65.0, 8.25]);
        assert_eq!(
            game.cross_passage(PassageId::from_raw(9)),
            Err(PassageCrossError::UnknownPassage)
        );
        game.riding = Some(5);
        assert_eq!(
            game.cross_passage(PassageId::from_raw(1)),
            Err(PassageCrossError::Riding)
        );
        game.riding = None;
        let player = game.player.as_ref().unwrap();
        assert_eq!(player.position, Vec3::new(0.5, 65.0, 8.25));
        assert!(sent(&mut reader).is_empty());

        assert_eq!(game.cross_passage(PassageId::from_raw(1)), Ok(()));
        let reason = refusal_code(PassageCrossRefusal::Vetoed);
        let refused = Message::Refused { id: 1, reason };
        game.remote_passage_sync(&encode(&refused).unwrap());
        assert_eq!(
            game.last_passage_refusal(),
            Some((PassageId::from_raw(1), PassageCrossRefusal::Vetoed))
        );
        // The correction that follows puts the player back where the server has it.
        game.handle_packet(C::PlayerPosition(play::PlayerPosition {
            teleport: 2,
            position: [0.5, 65.0, 8.25],
            delta: [0.0; 3],
            yaw: 0.0,
            pitch: 0.0,
            relatives: 0,
        }))
        .unwrap();
        let player = game.player.as_ref().unwrap();
        assert_eq!(player.position, Vec3::new(0.5, 65.0, 8.25));
        assert_eq!(game.cross_passage(PassageId::from_raw(1)), Ok(()));
        assert_eq!(game.last_passage_refusal(), None);
    });
}

/// Cancels every custom payload the window sends.
struct Silence;
#[mod_api::__subscriptions]
impl mod_api::Hooks for Silence {
    fn client_packet_send(
        &self,
        _: &dyn mod_api::ClientHost,
        e: &mut mod_api::ClientPacketSendEvent<'_>,
    ) -> mod_api::Flow {
        if e.packet.id.path() == "custom_payload" {
            mod_api::Flow::Cancel
        } else {
            mod_api::Flow::Continue
        }
    }
}

/// A mod cancels the crossing's payload: `Cancelled`, the player stays as it was, nothing
/// waits, so the next crossing is tried afresh (and cancelled again, still not `Pending`).
#[test]
#[ignore = "daily"]
fn a_crossing_a_mod_cancels_on_the_wire_moves_nothing() {
    let mods = vec![mod_api::ModEntry::new("silence", Box::new(Silence))];
    mod_platform::testing::with_mods(mods.into(), || {
        with_client_game(|game| {
            let (mut reader, _server) = scene(game, [0.5, 65.0, 8.25]);
            let player = game.player.as_mut().unwrap();
            player.velocity = Vec3::new(0.0, -0.0784, 0.25);
            game.previous_position = Vec3::new(0.5, 65.0, 8.0);
            for _ in 0..2 {
                assert_eq!(
                    game.cross_passage(PassageId::from_raw(1)),
                    Err(PassageCrossError::Cancelled)
                );
                let player = game.player.as_ref().unwrap();
                assert_eq!(player.position, Vec3::new(0.5, 65.0, 8.25));
                assert_eq!(player.velocity, Vec3::new(0.0, -0.0784, 0.25));
                assert_eq!(game.previous_position, Vec3::new(0.5, 65.0, 8.0));
                assert_eq!(game.yaw, 0.0);
                assert_eq!(game.passages.crossing, None);
            }
            assert!(
                !sent(&mut reader).iter().any(
                    |p| matches!(p, S::CustomPayload(p) if p.channel == "horizon:passage_cross")
                )
            );
        });
    });
}
