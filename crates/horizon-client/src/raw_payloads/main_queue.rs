//! A receiving main provider reserves capacity before enqueue_work reports Queued.
use super::{Delivery, Endpoint, Incoming};
use crate::{
    lan::NetEvent,
    net::{
        Event,
        ingress::{Prepared, Producer, Refusal, Refused},
    },
};
use mod_api::PayloadContextError;
use std::sync::{Arc, Mutex};

pub enum MainQueue {
    Client(Producer<Event>),
    /// A closed bundle's application payload. Its main events are staged into
    /// the bundle's batch instead of the ordinary ingress stream, so a payload
    /// received between delimiters is published only inside `Event::Bundle`.
    ClientBundled {
        events: Producer<Event>,
        batch: Arc<Mutex<Vec<Event>>>,
    },
    Server {
        events: Producer<NetEvent>,
        endpoint: Endpoint,
        conn: Option<u32>,
    },
}
impl MainQueue {
    /// Stage a bundled payload's main publications for its closing bundle.
    pub(crate) fn client_bundled(events: Producer<Event>, batch: Arc<Mutex<Vec<Event>>>) -> Self {
        Self::ClientBundled { events, batch }
    }
}
pub(crate) enum PreparedMain {
    Client(Prepared<Event>),
    ClientBundled(BundledMain),
    Server(Prepared<NetEvent>),
}
/// A reserved main event that joins a bundle's batch when the receiving root
/// commits. The reservation is released when the value moves into the batch.
pub(crate) struct BundledMain {
    prepared: Prepared<Event>,
    batch: Arc<Mutex<Vec<Event>>>,
}
/// Keeps failed opaque work alive until after the caller releases its epoch gate.
/// Boxing occurs only on failed publication, adding no allocation to successful publication.
pub(crate) enum FailedMain {
    Client(Box<Refused<Event>>),
    Server(Box<Refused<NetEvent>>),
}
impl FailedMain {
    pub fn reason(&self) -> Refusal {
        match self {
            Self::Client(error) => error.reason,
            Self::Server(error) => error.reason,
        }
    }
}
impl MainQueue {
    pub(crate) fn prepare(&self, incoming: Incoming) -> Result<PreparedMain, PayloadContextError> {
        let refusal = |reason| match reason {
            Refusal::Pressure => PayloadContextError::Backpressure,
            Refusal::Closed => PayloadContextError::Disconnected,
        };
        match self {
            Self::Client(events) => events
                .prepare(Event::RawPayload(Box::new(incoming)))
                .map(PreparedMain::Client)
                .map_err(|error| refusal(error.reason)),
            Self::ClientBundled { events, batch } => events
                .prepare(Event::RawPayload(Box::new(incoming)))
                .map(|prepared| {
                    PreparedMain::ClientBundled(BundledMain {
                        prepared,
                        batch: batch.clone(),
                    })
                })
                .map_err(|error| refusal(error.reason)),
            Self::Server {
                events,
                endpoint,
                conn,
            } => events
                .prepare(NetEvent::RawPayload(Box::new(Delivery {
                    conn: *conn,
                    endpoint: endpoint.clone(),
                    incoming,
                })))
                .map(PreparedMain::Server)
                .map_err(|error| refusal(error.reason)),
        }
    }
}
impl PreparedMain {
    pub(crate) fn publish(self) -> Result<(), FailedMain> {
        match self {
            Self::Client(prepared) => prepared
                .publish()
                .map_err(|error| FailedMain::Client(Box::new(error))),
            Self::ClientBundled(bundled) => {
                let event = bundled.prepared.defuse();
                bundled.batch.lock().unwrap().push(event);
                Ok(())
            }
            Self::Server(prepared) => prepared
                .publish()
                .map_err(|error| FailedMain::Server(Box::new(error))),
        }
    }
}
