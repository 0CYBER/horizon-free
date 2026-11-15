use super::Side;
use horizon_protocol::{buf::Writer, ids, payload_negotiation::Phase};
use mod_api::{PayloadBytes, PayloadId};

/// Encode vanilla packet id + declared channel + exact body, with no mod envelope.
/// Call only after resolving the declaration against the connection's frozen table.
pub fn encode(phase: Phase, side: Side, id: &PayloadId, bytes: PayloadBytes) -> Vec<u8> {
    let packet = packet_id(phase, side);
    let mut body = Writer::new();
    body.varint(packet);
    body.string(&id.0);
    body.data.extend_from_slice(&bytes.0);
    body.data
}

/// Exact uncompressed packet length without copying application bytes.
pub fn encoded_len(phase: Phase, side: Side, id: &PayloadId, len: usize) -> usize {
    varint_len(packet_id(phase, side) as usize)
        .saturating_add(varint_len(id.0.len()))
        .saturating_add(id.0.len())
        .saturating_add(len)
}
fn varint_len(mut value: usize) -> usize {
    let mut bytes = 1;
    while value >= 128 {
        value >>= 7;
        bytes += 1;
    }
    bytes
}
fn packet_id(phase: Phase, side: Side) -> i32 {
    match (phase, side) {
        (Phase::Configuration, Side::Client) => ids::configuration::serverbound::CUSTOM_PAYLOAD,
        (Phase::Configuration, Side::Server) => ids::configuration::clientbound::CUSTOM_PAYLOAD,
        (Phase::Play, Side::Client) => ids::play::serverbound::CUSTOM_PAYLOAD,
        (Phase::Play, Side::Server) => ids::play::clientbound::CUSTOM_PAYLOAD,
    }
}
