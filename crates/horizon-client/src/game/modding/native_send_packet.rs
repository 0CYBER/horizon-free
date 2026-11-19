//! `player_send_packet`: a server mod's clientbound play packet, sent as it is to one player.
use super::Game;
use horizon_protocol::packets::play::Clientbound as C;
use mod_api::{ClientboundPlayPacket, PlayerRef, SendPacketError};

impl Game<'_> {
    /// Sends `packet` to that player's client as it is (`ServerPlayer.connection.send`),
    /// once its body decodes whole as the named clientbound play packet. See the declaration
    /// in `host/network.rs` for the errors.
    pub(super) fn player_send_packet(
        &mut self,
        player: PlayerRef,
        packet: ClientboundPlayPacket,
    ) -> Result<(), SendPacketError> {
        if self.remote.is_some() {
            return Err(SendPacketError::NotConnected);
        }
        let reachable = self.native_acting_player() == Some(player)
            || self.online_player_record(player).is_some();
        if !reachable {
            return Err(SendPacketError::NotConnected);
        }
        let decoded = decode(packet)?;
        if self.native_send(player.uuid(), decoded) {
            Ok(())
        } else {
            Err(SendPacketError::NotConnected)
        }
    }
}

/// The packet the name and body make, as the player's client would read it.
fn decode(packet: ClientboundPlayPacket) -> Result<C, SendPacketError> {
    let id = match packet.id.namespace() {
        "minecraft" => horizon_protocol::ids::play::clientbound::NAMES
            .iter()
            .position(|name| *name == packet.id.path()),
        _ => None,
    }
    .ok_or(SendPacketError::UnknownPacket {
        id: packet.id.clone(),
    })?;
    let decoded =
        C::decode(id as i32, &packet.body).map_err(|error| match error.payload_limit() {
            Some(detail) => SendPacketError::PayloadTooLarge {
                size: detail.size as u64,
                limit: detail.limit as u64,
            },
            None => SendPacketError::InvalidBody,
        })?;
    // The engine's own channels carry its state; a mod's bytes there would be read as it.
    if let C::CustomPayload(payload) = &decoded
        && payload.channel.starts_with("horizon:")
    {
        return Err(SendPacketError::InvalidBody);
    }
    Ok(decoded)
}

#[cfg(test)]
mod tests {
    use super::{C, ClientboundPlayPacket, SendPacketError, decode};
    use horizon_protocol::buf::Writer;
    use mod_api::Identifier;

    fn packet(id: &str, body: Vec<u8>) -> ClientboundPlayPacket {
        ClientboundPlayPacket::new(Identifier::parse(id).unwrap(), body)
    }
    fn payload(channel: &str, len: usize) -> ClientboundPlayPacket {
        let mut body = Writer::new();
        body.string(channel);
        body.bytes(&vec![0; len]);
        packet("minecraft:custom_payload", body.data)
    }

    /// The body must decode whole as the named packet; a custom payload keeps vanilla's
    /// clientbound limit and stays off the engine's channels.
    #[test]
    fn a_mod_packet_is_checked_as_the_client_reads_it() {
        let mut ping = Writer::new();
        ping.i32(7);
        assert!(matches!(
            decode(packet("minecraft:ping", ping.data.clone())),
            Ok(C::Ping(_))
        ));
        ping.bytes(&[0]);
        assert_eq!(
            decode(packet("minecraft:ping", ping.data)).err(),
            Some(SendPacketError::InvalidBody)
        );
        for unknown in ["minecraft:no_such_packet", "test:ping"] {
            assert_eq!(
                decode(packet(unknown, Vec::new())).err(),
                Some(SendPacketError::UnknownPacket {
                    id: Identifier::parse(unknown).unwrap()
                })
            );
        }
        let limit = horizon_protocol::packets::custom_payload::CLIENTBOUND_LIMIT;
        assert!(decode(payload("voicechat:secret", limit)).is_ok());
        assert!(matches!(
            decode(payload("voicechat:secret", limit + 1)),
            Err(SendPacketError::PayloadTooLarge { .. })
        ));
        assert_eq!(
            decode(payload("horizon:mod_net", 1)).err(),
            Some(SendPacketError::InvalidBody)
        );
    }
}
