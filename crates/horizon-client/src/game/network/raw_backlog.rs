//! Transport-owned credit for ordered remote backlog slots.
use super::remote_chunks::ChunkFormat;
use horizon_client::net::Event;
use std::sync::Arc;

/// What the network thread delivered, in order: an event, or a chunk packet
/// handed to the decoder (with the format it decodes with), whose column
/// comes back from `ChunkDecoder::next`.
///
/// A closed bundle is one `Bundle`: its chunks are already submitted, and the
/// group is released only once every one of them can decode. Individual items
/// carry no lease when they belong to a bundle; the group holds the original
/// ingress credit until its last item is applied.
pub(super) enum Received {
    Event(Event, Option<horizon_client::net::ingress::ReceiveLease<Event>>),
    Chunk(
        Arc<ChunkFormat>,
        Option<horizon_client::net::ingress::ReceiveLease<Event>>,
    ),
    Bundle(
        Vec<BundlePart>,
        horizon_client::net::ingress::ReceiveLease<Event>,
    ),
}

pub(super) enum BundlePart {
    Event(Event),
    Chunk(Arc<ChunkFormat>),
}

// Keep net::Event's transport-owned staging slot bound honest as the actor evolves.
const _: () = assert!(
    std::mem::size_of::<Received>()
        <= std::mem::size_of::<(
            Event,
            horizon_client::net::ingress::ReceiveLease<Event>,
            usize,
            usize,
        )>()
);

impl Received {
    pub(super) fn into_event_credit(
        self,
    ) -> Option<horizon_client::net::ingress::ReceiveLease<Event>> {
        match self {
            Self::Event(_, credit) => credit,
            _ => unreachable!(),
        }
    }
}
