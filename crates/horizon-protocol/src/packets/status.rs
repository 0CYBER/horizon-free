//! Status state: server list ping.
use crate::{
    buf::{Long, Str},
    packet, packet_enum,
    packets::Empty,
};

packet!(StatusResponse { json: String => Str });
packet!(Ping { time: i64 => Long });

packet_enum!(Clientbound, crate::ids::status::clientbound {
    StatusResponse(StatusResponse) = STATUS_RESPONSE,
    PongResponse(Ping) = PONG_RESPONSE,
});
packet_enum!(Serverbound, crate::ids::status::serverbound {
    StatusRequest(Empty) = STATUS_REQUEST,
    PingRequest(Ping) = PING_REQUEST,
});

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packets::round_trip;

    #[test]
    fn status_round_trips() {
        round_trip(
            Clientbound::StatusResponse(StatusResponse {
                json: "{\"version\":{}}".into(),
            }),
            Clientbound::encode,
            Clientbound::decode,
        );
        round_trip(
            Clientbound::PongResponse(Ping { time: 123 }),
            Clientbound::encode,
            Clientbound::decode,
        );
        round_trip(
            Serverbound::StatusRequest(Empty),
            Serverbound::encode,
            Serverbound::decode,
        );
        round_trip(
            Serverbound::PingRequest(Ping { time: -5 }),
            Serverbound::encode,
            Serverbound::decode,
        );
    }
}
