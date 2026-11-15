use super::*;
use horizon_protocol::{
    buf::Writer,
    connection::{Connection, Pipe},
};
use mod_api::{Identifier, SendPacketError, ServerboundPlayPacket};
use std::time::Duration;

fn connected() -> (Session, Connection) {
    let (a, b) = Pipe::pair();
    let (_, physical) = Connection::local(a).split();
    let writer = QueuedWriter::new(physical).unwrap();
    let mut peer = Connection::local(b);
    peer.reader
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    writer
        .transition_protocol(horizon_protocol::payload_negotiation::Phase::Play, &[3])
        .unwrap();
    assert_eq!(peer.recv().unwrap(), (3, vec![]));
    let session = Session::replay("packet boundary");
    *session.writer.lock().unwrap() = Some(writer);
    (session, peer)
}
fn plugin(channel: &str, bytes: &[u8]) -> ServerboundPlayPacket {
    let mut body = Writer::new();
    body.string(channel);
    body.bytes(bytes);
    ServerboundPlayPacket::new(
        Identifier::parse("minecraft:custom_payload").unwrap(),
        body.data,
    )
}
#[test]
fn generic_submission_requires_a_physical_writer_and_known_packet_type() {
    let session = Session::replay("no physical writer");
    assert!(matches!(
        session.prepare_mod_packet(plugin("example:unregistered", &[7])),
        Err(SendPacketError::NotConnected)
    ));
    for name in ["example:pong", "minecraft:missing_packet"] {
        let id = Identifier::parse(name).unwrap();
        assert!(
            matches!(session.prepare_mod_packet(ServerboundPlayPacket::new(id.clone(), Vec::new())), Err(SendPacketError::UnknownPacket { id: refused }) if refused == id)
        );
    }
}
#[test]
fn generic_submission_preserves_plugin_bytes_and_rejects_malformed_packets() {
    let (session, mut peer) = connected();
    let bytes = vec![0x91; 32_767];
    session
        .prepare_mod_packet(plugin("voicechat:request_secret", &bytes))
        .unwrap()
        .publish();
    let (id, body) = peer.recv().unwrap();
    let play::Serverbound::CustomPayload(payload) = play::Serverbound::decode(id, &body).unwrap()
    else {
        panic!("not custom payload")
    };
    assert_eq!(payload.channel, "voicechat:request_secret");
    assert_eq!(payload.data, bytes);
    assert!(matches!(
        session.prepare_mod_packet(plugin("voicechat:request_secret", &vec![0; 32_768])),
        Err(SendPacketError::PayloadTooLarge {
            size: 32_768,
            limit: 32_767
        })
    ));
    let packet =
        |body| ServerboundPlayPacket::new(Identifier::parse("minecraft:pong").unwrap(), body);
    assert!(matches!(
        session.prepare_mod_packet(packet(vec![0, 0, 0])),
        Err(SendPacketError::InvalidBody)
    ));
    assert!(matches!(
        session.prepare_mod_packet(packet(vec![0, 0, 0, 7, 0])),
        Err(SendPacketError::InvalidBody)
    ));
    session
        .prepare_mod_packet(packet(vec![0, 0, 0, 7]))
        .unwrap()
        .publish();
    assert_eq!(
        peer.recv().unwrap(),
        (
            horizon_protocol::ids::play::serverbound::PONG,
            vec![0, 0, 0, 7]
        )
    );
}
#[test]
fn generic_submission_cannot_bypass_registered_or_engine_channel_authority() {
    crate::raw_payloads::test_support::with_peer(|_server_transport| {
        let (mut session, _peer) = connected();
        session.payloads =
            crate::raw_payloads::Transport::capture(crate::raw_payloads::Side::Client).unwrap();
        for channel in [
            crate::raw_payloads::test_support::CHANNEL,
            horizon_protocol::payload_negotiation::CHANNEL,
            "minecraft:brand",
            "horizon:recipe_book_settings",
        ] {
            // Registration protection takes precedence over fallback byte limits.
            let result = session.prepare_mod_packet(plugin(channel, &vec![0; 1 << 20]));
            assert!(
                matches!(&result, Err(SendPacketError::ProtectedPayload { id }) if id.0 == channel),
                "channel {channel}: {:?}",
                result.err()
            );
        }
    });
}
#[test]
fn generic_reservations_refuse_pressure_before_publication_and_cancel_refunds() {
    let (session, mut peer) = connected();
    let mut prepared = Vec::new();
    let bytes = vec![0x42; 32_767];
    let mut full = false;
    for _ in 0..600 {
        match session.prepare_mod_packet(plugin("example:unregistered", &bytes)) {
            Ok(packet) => prepared.push(packet),
            Err(SendPacketError::Backpressure) => {
                full = true;
                break;
            }
            Err(error) => panic!("unexpected refusal: {error:?}"),
        }
    }
    assert!(full, "finite owned-byte quota must refuse reservations");
    peer.reader
        .set_read_timeout(Some(Duration::from_millis(20)))
        .unwrap();
    assert!(
        peer.recv().is_err(),
        "prepared packets leaked before root publication"
    );
    drop(prepared);
    peer.reader
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    session
        .prepare_mod_packet(plugin("example:unregistered", &bytes))
        .unwrap()
        .publish();
    let (id, body) = peer.recv().unwrap();
    let play::Serverbound::CustomPayload(payload) = play::Serverbound::decode(id, &body).unwrap()
    else {
        panic!("not custom payload")
    };
    assert_eq!(payload.data, bytes);
}
#[test]
fn generic_prepared_packet_cannot_cross_a_configuration_round_trip() {
    use horizon_protocol::payload_negotiation::Phase;
    let (session, mut peer) = connected();
    let prepared = session
        .prepare_mod_packet(plugin("example:unregistered", &[7]))
        .unwrap();
    let writer = session.writer.lock().unwrap().as_ref().unwrap().clone();
    writer
        .transition_protocol(Phase::Configuration, &[8])
        .unwrap();
    assert_eq!(peer.recv().unwrap(), (8, vec![]));
    assert!(matches!(
        session.prepare_mod_packet(plugin("example:unregistered", &[9])),
        Err(SendPacketError::NotInPlay)
    ));
    writer.transition_protocol(Phase::Play, &[10]).unwrap();
    assert_eq!(peer.recv().unwrap(), (10, vec![]));
    prepared.publish();
    peer.reader
        .set_read_timeout(Some(Duration::from_millis(20)))
        .unwrap();
    assert!(
        peer.recv().is_err(),
        "boolean Play state hid a stale protocol incarnation"
    );
}
