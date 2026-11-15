//! Connection-owned frozen codec identities, separate from peer negotiation.
use std::collections::HashSet;

/// The protocol state a local custom-payload codec can receive in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PayloadProtocol {
    /// Login's configuration state.
    Configuration,
    /// Joined gameplay.
    Play,
}

/// The receiving direction of a custom-payload codec.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PayloadFlow {
    /// Server to client.
    ToClient,
    /// Client to server.
    ToServer,
}

/// Immutable local codec identities. This does not authorize sending to a peer:
/// negotiation and the connection's current epoch must still admit that send.
/// An empty snapshot allocates no collections and retains engine codecs only.
#[derive(Clone, Debug, Default)]
pub struct PayloadCodecs {
    channels: [HashSet<String>; 4],
}

impl PayloadCodecs {
    /// Freeze identities already validated by their owning registration boundary.
    pub fn from_channels(
        entries: impl IntoIterator<Item = (String, PayloadProtocol, PayloadFlow)>,
    ) -> Self {
        let mut result = Self::default();
        for (id, protocol, flow) in entries {
            let index = Self::index(
                protocol == PayloadProtocol::Play,
                flow == PayloadFlow::ToClient,
            );
            result.channels[index].insert(id);
        }
        result
    }

    fn index(play: bool, clientbound: bool) -> usize {
        usize::from(play) * 2 + usize::from(clientbound)
    }

    pub(super) fn contains(&self, id: &str, play: bool, clientbound: bool) -> bool {
        super::uses_registered_payload_codec(id, play, clientbound)
            || self.channels[Self::index(play, clientbound)].contains(id)
    }
}
