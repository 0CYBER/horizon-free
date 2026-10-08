//! Handshake state: ClientIntentionPacket.
use crate::{
    buf::{StrN, UShort, VarInt},
    packet, packet_enum,
};

/// ClientIntent ids.
pub const STATUS: i32 = 1;
pub const LOGIN: i32 = 2;
pub const TRANSFER: i32 = 3;

packet!(Intention {
    protocol: i32 => VarInt,
    host: String => StrN<255>,
    port: u16 => UShort,
    intent: i32 => VarInt,
});

packet_enum!(Serverbound, crate::ids::handshake::serverbound { Intention(Intention) = INTENTION });

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intention_bytes() {
        let p = Serverbound::Intention(Intention {
            protocol: 777,
            host: "localhost".into(),
            port: 25565,
            intent: LOGIN,
        });
        let body = p.encode();
        assert_eq!(body[0], 0);
        assert_eq!(&body[1..3], &[0x89, 0x06]);
        assert_eq!(&body[body.len() - 3..], &[0x63, 0xdd, 2]);
        crate::packets::round_trip(p, Serverbound::encode, Serverbound::decode);
    }
}
