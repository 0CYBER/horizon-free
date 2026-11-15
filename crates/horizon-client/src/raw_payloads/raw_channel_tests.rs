//! Raw plugin channels against a vanilla-shaped peer (no Horizon offer): sends,
//! fan-out to every declaring mod in load order, the vanilla size limits in both
//! directions, undeclared channels and the load-time collision refusal.
use super::*;
use crate::lan::{NetEvent, PlayerSender};
use horizon_protocol::packets::{custom_payload::CustomPayload, play};
use mod_api::{Hooks, IPayloadContext, IPayloadHandler, ModEntry, PayloadBytes, Registrar};

const CHANNEL: &str = "voicechat:secret";
const SEND_ONLY: &str = "bungeecord:main";

struct Listener;
impl IPayloadHandler for Listener {
    fn handle(
        &self,
        _: PayloadBytes,
        _: &mut dyn IPayloadContext,
    ) -> Result<(), mod_api::PayloadHandleError> {
        Ok(())
    }
}
struct Declares {
    receives: bool,
    send_only: bool,
}
#[mod_api::__subscriptions]
impl Hooks for Declares {
    fn register(&self, registrar: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        let mut payloads = registrar.payload_handlers();
        let mut raw = payloads.registrar("1");
        if self.receives {
            raw.play_raw(PayloadId(CHANNEL.into()), Some(Arc::new(Listener)));
        }
        if self.send_only {
            raw.play_raw(PayloadId(SEND_ONLY.into()), None);
        }
    }
}
struct Typed;
impl IPayloadHandler for Typed {
    fn handle(
        &self,
        _: PayloadBytes,
        _: &mut dyn IPayloadContext,
    ) -> Result<(), mod_api::PayloadHandleError> {
        Ok(())
    }
}
#[mod_api::__subscriptions]
impl Hooks for Typed {
    fn register(&self, registrar: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        registrar
            .payload_handlers()
            .registrar("1")
            .play_to_server(PayloadId(CHANNEL.into()), Arc::new(Typed));
    }
}

/// Two receiving mods and a send-only one, with a vanilla peer admitted
/// without an offer, as `finish_without_offer` admits it.
fn with_vanilla_peer(run: impl FnOnce(Arc<Transport>)) {
    let mods = vec![
        ModEntry::new(
            "voicechat",
            Box::new(Declares {
                receives: true,
                send_only: false,
            }),
        ),
        ModEntry::new(
            "voicebridge",
            Box::new(Declares {
                receives: true,
                send_only: true,
            }),
        ),
    ];
    mod_platform::testing::with_mods(mods.into(), || {
        mod_platform::__private::part_tests::install_parts_fixture(mod_api::__private::mods())
            .unwrap();
        let transport = Transport::capture(Side::Server).unwrap();
        assert!(
            transport.local.manifest.entries().is_empty(),
            "raw channels are never offered"
        );
        assert!(transport.connection.is_some(), "raw-only still keeps state");
        transport.accept(&[]).unwrap();
        transport.finish_configuration().unwrap();
        run(transport)
    });
}

fn serverbound(channel: &str, len: usize) -> CustomPayload {
    CustomPayload {
        channel: channel.into(),
        data: vec![7; len],
    }
}

#[test]
fn a_raw_channel_reaches_every_declaring_mod_in_load_order() {
    with_vanilla_peer(|transport| {
        let (sender, _out) = PlayerSender::payload_executor_test_sender(transport.clone());
        let (events, receiver) = crate::lan::event_channel_with_capacity(1 << 20);
        let Ok(Classified::Raw(deliveries)) =
            Incoming::classify(&transport, Phase::Play, serverbound(CHANNEL, 5))
        else {
            panic!("a declared raw channel is the mods'");
        };
        for incoming in deliveries {
            assert!(incoming.raw);
            let endpoint = Endpoint::server(sender.clone());
            dispatch(
                incoming,
                endpoint.clone(),
                MainQueue::Server {
                    events: events.clone(),
                    endpoint,
                    conn: Some(1),
                },
            );
        }
        let owners: Vec<_> = receiver
            .take_turn()
            .into_iter()
            .map(|receipt| match receipt.into_parts().0 {
                NetEvent::RawPayload(delivery) => {
                    let Body::Payload(bytes) = &delivery.incoming.body else {
                        panic!("a message, not work");
                    };
                    assert_eq!(bytes.0, vec![7; 5]);
                    delivery.incoming.receiver.owner()
                }
                _ => panic!("only raw deliveries were queued"),
            })
            .collect();
        assert_eq!(owners, ["voicechat", "voicebridge"]);
        // Undeclared and send-only channels keep their unrelated-plugin behaviour.
        for channel in [SEND_ONLY, "other:channel"] {
            assert!(matches!(
                Incoming::classify(&transport, Phase::Play, serverbound(channel, 1)),
                Ok(Classified::Other(_))
            ));
        }
    });
}

#[test]
fn raw_sends_reach_a_vanilla_peer_within_the_clientbound_limit() {
    with_vanilla_peer(|transport| {
        let (sender, out) = PlayerSender::payload_executor_test_sender(transport.clone());
        let endpoint = Endpoint::server(sender);
        let epoch = transport.epoch().unwrap();
        let id = PayloadId(SEND_ONLY.into());
        let body = b"\x00\x07Connect\x00\x05lobby".to_vec();
        endpoint
            .prepare(&epoch, &id, PayloadBytes(body.clone()))
            .unwrap()
            .publish()
            .unwrap();
        // Play clientbound custom_payload (0x18), the channel as a length-prefixed string,
        // then the body with no envelope: written out by hand, not by the encoder under test.
        let mut expected = vec![0x18, 15];
        expected.extend_from_slice(b"bungeecord:main");
        expected.extend_from_slice(&body);
        assert!(out().contains(&expected));
        // ClientboundCustomPayloadPacket's limit, as a literal so a changed constant fails here.
        assert!(
            endpoint
                .prepare(&epoch, &id, PayloadBytes(vec![0; 1_048_576]))
                .is_ok()
        );
        assert_eq!(
            endpoint
                .prepare(&epoch, &id, PayloadBytes(vec![0; 1_048_577]))
                .err(),
            Some(PayloadSendError::PayloadTooLarge {
                id: id.clone(),
                size: 1_048_577,
                limit: 1_048_576,
            })
        );
        let undeclared = PayloadId("other:channel".into());
        assert_eq!(
            endpoint
                .prepare(&epoch, &undeclared, PayloadBytes(vec![1]))
                .err(),
            Some(PayloadSendError::UnknownPayload(undeclared))
        );
    });
}

#[test]
fn a_raw_channel_keeps_the_vanilla_serverbound_limit() {
    with_vanilla_peer(|transport| {
        let decode = |len: usize| {
            let mut body = horizon_protocol::buf::Writer::new();
            body.string(CHANNEL);
            body.bytes(&vec![0; len]);
            play::Serverbound::decode_with_payload_codecs(
                horizon_protocol::ids::play::serverbound::CUSTOM_PAYLOAD,
                &body.data,
                &transport.local.codecs,
            )
        };
        // DiscardedPayload's serverbound limit, as a literal.
        assert!(decode(32_767).is_ok());
        assert!(decode(32_768).is_err());
    });
}

#[test]
fn a_raw_declaration_colliding_with_a_typed_payload_refuses_load() {
    let mods = vec![
        ModEntry::new("voicechat", Box::new(Typed)),
        ModEntry::new(
            "voicebridge",
            Box::new(Declares {
                receives: true,
                send_only: false,
            }),
        ),
    ];
    mod_platform::testing::with_mods(mods.into(), || {
        let refused =
            mod_platform::__private::part_tests::install_parts_fixture(mod_api::__private::mods())
                .unwrap_err();
        assert!(
            refused.iter().any(|line| line.contains("voicebridge")
                && line.contains(CHANNEL)
                && line.contains("voicechat")),
            "{refused:?}"
        );
    });
}
