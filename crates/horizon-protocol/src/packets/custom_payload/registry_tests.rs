use super::{CustomPayload, PayloadCodecs, PayloadFlow, PayloadProtocol};
use crate::{
    buf::Reader,
    packets::{configuration, play},
};

#[test]
fn registered_bodies_are_not_subject_to_discard_or_network_frame_caps_in_the_codec() {
    // Memory connections have no framing cap. The network service, not the raw
    // registered codec, must refuse a packet that cannot fit its active framing.
    let bytes = vec![0x39; crate::frame::MAX_UNCOMPRESSED + 1];
    macro_rules! check {
        ($module:ident, $direction:ident, $phase:expr, $flow:expr) => {{
            let packet = $module::$direction::CustomPayload(CustomPayload {
                channel: "example:raw".into(),
                data: bytes.clone(),
            });
            let codecs = PayloadCodecs::from_channels([("example:raw".into(), $phase, $flow)]);
            assert!(packet.try_encode().is_err());
            let encoded = packet.try_encode_with_payload_codecs(&codecs).unwrap();
            let mut reader = Reader::new(&encoded);
            let id = reader.varint().unwrap();
            assert_eq!(
                $module::$direction::decode_with_payload_codecs(id, reader.rest(), &codecs)
                    .unwrap(),
                packet,
            );
            // The opposite direction must still use the discard fallback.
            let wrong = PayloadCodecs::from_channels([(
                "example:raw".into(),
                $phase,
                if $flow == PayloadFlow::ToClient {
                    PayloadFlow::ToServer
                } else {
                    PayloadFlow::ToClient
                },
            )]);
            assert!(packet.try_encode_with_payload_codecs(&wrong).is_err());
            assert!(
                $module::$direction::decode_with_payload_codecs(id, reader.rest(), &wrong).is_err()
            );
            let wrong_phase = PayloadCodecs::from_channels([(
                "example:raw".into(),
                if $phase == PayloadProtocol::Play {
                    PayloadProtocol::Configuration
                } else {
                    PayloadProtocol::Play
                },
                $flow,
            )]);
            assert!(packet.try_encode_with_payload_codecs(&wrong_phase).is_err());
            assert!(
                $module::$direction::decode_with_payload_codecs(id, reader.rest(), &wrong_phase)
                    .is_err()
            );
        }};
    }
    check!(
        configuration,
        Serverbound,
        PayloadProtocol::Configuration,
        PayloadFlow::ToServer
    );
    check!(
        configuration,
        Clientbound,
        PayloadProtocol::Configuration,
        PayloadFlow::ToClient
    );
    check!(
        play,
        Serverbound,
        PayloadProtocol::Play,
        PayloadFlow::ToServer
    );
    check!(
        play,
        Clientbound,
        PayloadProtocol::Play,
        PayloadFlow::ToClient
    );
}
