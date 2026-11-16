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

pub enum MainQueue {
    Client(Producer<Event>),
    Server {
        events: Producer<NetEvent>,
        endpoint: Endpoint,
        conn: Option<u32>,
    },
}
pub(crate) enum PreparedMain {
    Client(Prepared<Event>),
    Server(Prepared<NetEvent>),
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
            Self::Server(prepared) => prepared
                .publish()
                .map_err(|error| FailedMain::Server(Box::new(error))),
        }
    }
}
