//! Engine declarations in the negotiated table: admitted like a mod's, received by the engine.
use super::*;
use horizon_protocol::{
    attachment_sync,
    packets::custom_payload::CustomPayload,
    payload_negotiation::{Control, Negotiation},
};

/// An installation with no mods: only the engine's declarations.
fn engine_only() -> Arc<Local> {
    let mut manifest = Manifest::builder();
    engine::register(&mut manifest).unwrap();
    Arc::new(Local::new(manifest.finish().unwrap(), None))
}

/// What a server accepts from a peer that offered `peer`, or from one that offered nothing.
fn accepted(local: &Local, peer: Option<&Manifest>) -> Vec<negotiation::Entry> {
    let mut negotiation = Negotiation::new(local.manifest.clone());
    match peer {
        Some(peer) => {
            negotiation.receive(Control::Offer(peer.clone())).unwrap();
        }
        None => negotiation.finish_without_offer().unwrap(),
    }
    negotiation.accepted().unwrap().to_vec()
}

/// A connection in play after accepting `entries`.
fn playing(local: &Arc<Local>, side: Side, entries: &[negotiation::Entry]) -> Arc<Transport> {
    let transport = Transport::new(local.clone(), side);
    transport.accept(entries).unwrap();
    transport.finish_configuration().unwrap();
    transport
}

fn sync_packet() -> CustomPayload {
    CustomPayload {
        channel: attachment_sync::CHANNEL.into(),
        data: vec![4, 0, 0],
    }
}

#[test]
fn engine_only_installation_has_no_mod_declarations_and_needs_no_mod_handler() {
    let local = engine_only();
    assert!(!local.has_mod_declarations());
    // A client receives the engine channel with no mod callback for it.
    let entries = accepted(&local, Some(&local.manifest));
    assert_eq!(entries.len(), 1);
    assert!(local.freeze(&entries, Side::Client).is_ok());
}

#[test]
fn a_horizon_peer_gets_the_channel_and_a_vanilla_peer_gets_unsupported_peer() {
    let local = engine_only();
    let id = PayloadId(attachment_sync::CHANNEL.into());
    let horizon = playing(
        &local,
        Side::Server,
        &accepted(&local, Some(&local.manifest)),
    );
    assert_eq!(
        horizon.epoch().unwrap().peer.can_send(Phase::Play, &id),
        Ok(())
    );
    // A vanilla client sends no Offer: the optional declaration is absent, not refused.
    let vanilla = playing(&local, Side::Server, &accepted(&local, None));
    assert_eq!(
        vanilla.epoch().unwrap().peer.can_send(Phase::Play, &id),
        Err(PayloadSendError::UnsupportedPeer(id.clone()))
    );
    // So does a Horizon build without the service: its Offer lacks the declaration.
    let older = playing(
        &local,
        Side::Server,
        &accepted(&local, Some(&Manifest::default())),
    );
    assert_eq!(
        older.epoch().unwrap().peer.can_send(Phase::Play, &id),
        Err(PayloadSendError::UnsupportedPeer(id))
    );
}

#[test]
fn a_negotiated_engine_message_reaches_the_engine_decoder_and_nothing_else_does() {
    let local = engine_only();
    let both = accepted(&local, Some(&local.manifest));
    let client = playing(&local, Side::Client, &both);
    assert!(matches!(
        Incoming::classify(&client, Phase::Play, sync_packet()),
        Ok(Classified::Other(packet)) if packet.channel == attachment_sync::CHANNEL
    ));
    // Not negotiated with this server: a protocol refusal, never applied.
    let absent = playing(&local, Side::Client, &accepted(&local, None));
    assert!(matches!(
        Incoming::classify(&absent, Phase::Play, sync_packet()),
        Err(Failure {
            error: PayloadSendError::UnsupportedPeer(_),
            ..
        })
    ));
    // A client sending the clientbound channel to a server is the wrong direction.
    let server = playing(&local, Side::Server, &both);
    assert!(matches!(
        Incoming::classify(&server, Phase::Play, sync_packet()),
        Err(Failure {
            error: PayloadSendError::WrongDirection(_),
            ..
        })
    ));
    // Play only: during configuration the channel is not the engine's.
    let configuring = Transport::new(local.clone(), Side::Client);
    configuring.accept(&both).unwrap();
    assert!(matches!(
        Incoming::classify(&configuring, Phase::Configuration, sync_packet()),
        Err(Failure {
            error: PayloadSendError::WrongProtocol(_),
            ..
        })
    ));
}

#[test]
fn a_send_is_bound_to_the_epoch_it_was_prepared_in() {
    let local = engine_only();
    let id = PayloadId(attachment_sync::CHANNEL.into());
    let transport = playing(
        &local,
        Side::Server,
        &accepted(&local, Some(&local.manifest)),
    );
    let old = transport.epoch().unwrap();
    // Reconfiguration: the old play epoch ends, the new one is configuration.
    transport.begin_configuration();
    transport
        .accept(&accepted(&local, Some(&local.manifest)))
        .unwrap();
    assert!(!old.active());
    let new = transport.epoch().unwrap();
    assert_eq!(new.phase, Phase::Configuration);
    assert_eq!(
        new.peer.can_send(new.phase, &id),
        Err(PayloadSendError::WrongProtocol(id))
    );
}
