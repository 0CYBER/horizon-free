//! Physical reply targets usable by either receiving executor without borrowing Game.
use super::{Side, Transport, connection::Epoch, encode, encoded_len};
use crate::{
    lan::{PlayerSender, config_payloads::ConfigSender},
    net::{Event, PreparedPayload, queued_writer::QueuedWriter},
};
use horizon_protocol::{connection::Framing, frame, payload_negotiation::Phase};
use mod_api::{PayloadBytes, PayloadFrameStage, PayloadId, PayloadSendError, Text};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

#[derive(Clone)]
pub struct Endpoint(Target);
#[derive(Clone)]
enum Target {
    Client {
        writer: Arc<Mutex<Option<QueuedWriter>>>,
        configuring: Arc<AtomicBool>,
        transport: Arc<Transport>,
        events: crate::net::ingress::Producer<Event>,
    },
    Server(PlayerSender),
    Configuration(ConfigSender),
}
/// Retains no endpoint, sender, producer, writer, transport or payload lease.
#[cfg(any(test, feature = "test-payload-probe"))]
pub(super) struct Observation {
    transport: std::sync::Weak<Transport>,
    side: Side,
    diagnostics: Box<dyn Fn() -> String + Send + Sync>,
}
#[cfg(any(test, feature = "test-payload-probe"))]
impl Observation {
    pub(super) fn same_target(&self, endpoint: &Endpoint) -> bool {
        self.transport.ptr_eq(&Arc::downgrade(endpoint.transport())) && self.side == endpoint.side()
    }
    pub(super) fn diagnostics(&self) -> String {
        let connection = match self.transport.upgrade() {
            Some(transport) => transport
                .connection
                .as_ref()
                .map(|c| c.diagnostics())
                .unwrap_or_else(|| "no connection".into()),
            None => "transport owner dropped".into(),
        };
        format!("{:?}: {connection}; {}", self.side, (self.diagnostics)())
    }
}
pub enum Prepared {
    Client(PreparedPayload),
    Server(crate::lan::conn::PreparedPayload),
}
impl Prepared {
    pub fn publish(self) -> Result<(), PayloadSendError> {
        match self {
            Self::Client(send) => send.publish(),
            Self::Server(send) => send.publish(),
        }
    }
}
impl Endpoint {
    pub(crate) fn client(
        writer: Arc<Mutex<Option<QueuedWriter>>>,
        configuring: Arc<AtomicBool>,
        transport: Arc<Transport>,
        events: crate::net::ingress::Producer<Event>,
    ) -> Self {
        Self(Target::Client {
            writer,
            configuring,
            transport,
            events,
        })
    }
    pub fn server(sender: PlayerSender) -> Self {
        Self(Target::Server(sender))
    }
    pub fn configuration(sender: ConfigSender) -> Self {
        Self(Target::Configuration(sender))
    }
    pub fn side(&self) -> Side {
        match self.0 {
            Target::Client { .. } => Side::Client,
            _ => Side::Server,
        }
    }
    pub fn transport(&self) -> &Arc<Transport> {
        match &self.0 {
            Target::Client { transport, .. } => transport,
            Target::Server(sender) => sender.payload_transport(),
            Target::Configuration(sender) => sender.payload_transport(),
        }
    }
    #[cfg(any(test, feature = "test-payload-probe"))]
    pub(super) fn observation(&self) -> Observation {
        let diagnostics: Box<dyn Fn() -> String + Send + Sync> = match &self.0 {
            Target::Client { writer, .. } => {
                let writer = Arc::downgrade(writer);
                Box::new(move || {
                    let Some(writer) = writer.upgrade() else {
                        return "writer handle owner dropped".into();
                    };
                    match writer.try_lock() {
                        Ok(w) => {
                            let current = w.clone();
                            drop(w);
                            current
                                .map(|w| w.diagnostics())
                                .unwrap_or_else(|| "writer absent".into())
                        }
                        Err(_) => "writer handle busy/poisoned".into(),
                    }
                })
            }
            Target::Server(sender) => sender.payload_diagnostic_observer(),
            Target::Configuration(_) => Box::new(|| "configuration sender not observed".into()),
        };
        Observation {
            transport: Arc::downgrade(self.transport()),
            side: self.side(),
            diagnostics,
        }
    }
    /// Peer authority is checked before bytes are copied. Publication belongs to the root.
    pub fn prepare(
        &self,
        epoch: &Arc<Epoch>,
        id: &PayloadId,
        bytes: PayloadBytes,
    ) -> Result<Prepared, PayloadSendError> {
        #[cfg(any(test, feature = "test-payload-probe"))]
        super::staging_probe::capture(&id.0, self);
        if !epoch.active() {
            return Err(PayloadSendError::StaleEpoch);
        }
        epoch.peer.can_send(epoch.phase, id)?;
        // A raw channel keeps vanilla's clientbound custom-payload limit, by name.
        let limit = horizon_protocol::packets::custom_payload::CLIENTBOUND_LIMIT;
        if epoch.peer.local.is_raw(&id.0) && bytes.0.len() > limit {
            return Err(PayloadSendError::PayloadTooLarge {
                id: id.clone(),
                size: bytes.0.len() as u64,
                limit: limit as u64,
            });
        }
        let len = encoded_len(epoch.phase, self.side(), id, bytes.0.len());
        match &self.0 {
            Target::Client {
                writer,
                configuring,
                ..
            } => {
                if configuring.load(Ordering::Acquire) != (epoch.phase == Phase::Configuration) {
                    return Err(PayloadSendError::WrongProtocol(id.clone()));
                }
                let writer = writer
                    .lock()
                    .unwrap()
                    .as_ref()
                    .cloned()
                    .ok_or(PayloadSendError::Disconnected)?;
                preflight(writer.framing(), writer.payload_capacity(), len, id)?;
                writer
                    .prepare_payload(encode(epoch.phase, self.side(), id, bytes), id)
                    .map(Prepared::Client)
            }
            Target::Server(sender) => {
                preflight(sender.payload_framing(), sender.payload_capacity(), len, id)?;
                sender
                    .prepare_payload(encode(epoch.phase, self.side(), id, bytes), id)
                    .map(Prepared::Server)
            }
            Target::Configuration(sender) => {
                preflight(sender.framing(), sender.payload_capacity(), len, id)?;
                sender
                    .prepare_payload(encode(epoch.phase, self.side(), id, bytes), id)
                    .map(Prepared::Client)
            }
        }
    }
    /// A failure closes this physical connection only; a client reason joins its ordered events.
    pub fn disconnect(&self, reason: Text) {
        let reason = crate::api_text::to_engine(&reason).unwrap_or_else(|error| {
            crate::text::Text::tr(
                "disconnect.genericReason",
                &[&format!("invalid payload disconnect reason: {error:?}")],
            )
        });
        self.transport().close();
        match &self.0 {
            Target::Client { writer, events, .. } => {
                let _ = events.send(Event::Disconnected {
                    reason: reason.plain(),
                    key: None,
                    from_server: false,
                });
                if let Some(writer) = writer.lock().unwrap().as_ref() {
                    writer.shutdown();
                }
            }
            Target::Server(sender) => {
                sender.disconnect(crate::lan::data::component_nbt(&reason.component()))
            }
            Target::Configuration(sender) => {
                sender.disconnect(crate::lan::data::component_nbt(&reason.component()))
            }
        }
    }
}
fn preflight(
    framing: Framing,
    capacity: usize,
    len: usize,
    id: &PayloadId,
) -> Result<(), PayloadSendError> {
    let frame_limit = match framing {
        Framing::Memory => None,
        Framing::Network(None) => Some((PayloadFrameStage::Encoded, frame::MAX_FRAME)),
        Framing::Network(Some(_)) => {
            Some((PayloadFrameStage::Uncompressed, frame::MAX_UNCOMPRESSED))
        }
    };
    if let Some((stage, limit)) = frame_limit
        && len > limit
    {
        return Err(PayloadSendError::FrameTooLarge {
            id: id.clone(),
            stage,
            size: len as u64,
            limit: limit as u64,
        });
    }
    if len > capacity {
        return Err(PayloadSendError::Backpressure);
    }
    Ok(())
}
