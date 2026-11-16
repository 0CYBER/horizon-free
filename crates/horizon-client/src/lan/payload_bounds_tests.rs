//! Registered engine payloads must retain their own structural or byte bounds.
use horizon_protocol::{
    self as protocol,
    buf::{Codec, Reader, Writer},
};

#[test]
fn every_registered_engine_codec_refuses_a_hostile_oversized_body() {
    type Rejects = fn(&[u8]) -> bool;
    let cases: [(&str, Rejects); 13] = [
        ("minecraft:brand", |body| {
            let mut packet = Writer::new();
            packet.string("minecraft:brand");
            packet.bytes(body);
            protocol::packets::custom_payload::ServerboundPayload::read(&mut Reader::new(
                &packet.data,
            ))
            .is_err()
        }),
        ("horizon:quiet_commands", |body| {
            protocol::quiet::decode(body).is_err()
        }),
        ("horizon:mod_list", |body| {
            protocol::mod_list::decode(body).is_err()
        }),
        ("horizon:attachment_sync", |body| {
            protocol::attachment_sync::decode(body).is_err()
        }),
        ("horizon:menu_extra", |body| {
            protocol::menu_extra::decode(body).is_err()
        }),
        ("horizon:anvil_cost", |body| {
            protocol::anvil_cost::decode(body).is_err()
        }),
        ("horizon:passage_sync", |body| {
            protocol::passage_sync::decode(body).is_err()
        }),
        ("horizon:passage_cross", |body| {
            protocol::passage_sync::decode_cross(body).is_err()
        }),
        ("horizon:lod", |body| {
            crate::lod_net::Offer::decode(body).is_none()
                && crate::lod_net::Request::decode(body).is_err()
                && crate::lod_net::decode_reply(body, &Default::default(), 1).is_err()
        }),
        ("horizon:block_registry", |body| {
            crate::content_identity::decode_payload(body).is_err()
        }),
        ("horizon:recipe_content", |body| {
            super::recipes::ContentPart::decode(body).is_err()
        }),
        ("horizon:recipe_transfer", |body| {
            super::recipes::decode_transfer_request(body).is_err()
                && super::recipes::decode_transfer_reply(body).is_err()
        }),
        ("horizon:recipe_book_settings", |body| {
            crate::recipe_book::settings_codec::decode(body).is_none()
        }),
    ];
    // Start with zeros: valid empty/fixed fields followed by excessive trailing
    // bytes, not just an impossible prefix which all parsers trivially reject.
    let mut body = vec![0; 2_097_152];
    for (channel, rejects) in cases {
        assert!(rejects(&body), "{channel} accepted an oversized body");
    }
    // A real LOD offer with trailing bytes must also fail. Its decoder used
    // to accept its prefix and ignore the rest (without allocating it).
    let mut offer = crate::lod_net::Offer::new(16).encode();
    offer.append(&mut body);
    assert!(crate::lod_net::Offer::decode(&offer).is_none());
}
