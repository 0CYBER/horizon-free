//! Owned application messages and work retain ingress credit through root completion.
use super::{
    Transport,
    connection::{Epoch, IngressLease, MainTask},
};
use horizon_protocol::{packets::custom_payload::CustomPayload, payload_negotiation::Phase};
use mod_api::{
    __private::game::PayloadReceiver, PayloadBytes, PayloadContextError, PayloadId,
    PayloadSendError, PayloadWork,
};
use std::sync::Arc;
use std::time::Instant;

pub enum Body {
    Payload(PayloadBytes),
    Work(Arc<dyn PayloadWork>),
}
pub struct Incoming {
    pub epoch: Arc<Epoch>,
    pub id: PayloadId,
    pub body: Body,
    pub receiver: PayloadReceiver,
    pub main: MainTask,
    /// Not refunded by moving from I/O to Hub, backlog, decoder barrier or executor.
    pub lease: Arc<IngressLease>,
    /// A queued work resource may retain its original payload buffer. Keep that
    /// original credit as well as this additional work record's envelope credit.
    pub work_lease: Option<IngressLease>,
    /// A raw channel's delivery to one of its declaring mods: a handler's
    /// error is that mod's fault, not the peer's, and doesn't end the session.
    pub raw: bool,
    pub(super) turn: Option<super::connection::PayloadTurn>,
    busy: Option<mod_api::PayloadBusy>,
}
const _: () = assert!(std::mem::size_of::<Incoming>() <= 256);
pub enum Classified {
    Application(Incoming),
    /// A raw channel's message, once for each declaring mod in load order.
    Raw(Vec<Incoming>),
    Other(CustomPayload),
}
#[derive(Debug)]
pub struct Failure {
    pub id: PayloadId,
    pub error: PayloadSendError,
    holder: Option<Box<mod_api::PayloadBusy>>,
}
impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(holder) = self.holder.as_ref() {
            write!(
                f,
                "payload {} handler failed at the inbound queue limit: {holder}",
                self.id.0
            )
        } else {
            write!(f, "payload {} refused: {:?}", self.id.0, self.error)
        }
    }
}
impl Failure {
    fn new(transport: &Transport, id: PayloadId, error: PayloadSendError) -> Self {
        let holder = if matches!(error, PayloadSendError::Backpressure) {
            transport
                .connection
                .as_ref()
                .and_then(|connection| connection.busy_holder())
        } else {
            None
        };
        Self {
            id,
            error,
            holder: holder.map(Box::new),
        }
    }
}
impl Incoming {
    /// Restore the exact buffer after a guest declined to enter its busy group.
    /// The original lease, phase barrier and deadline remain attached.
    pub fn defer(
        &mut self,
        payload: PayloadBytes,
        holder: &mod_api::PayloadBusy,
        now: Instant,
    ) -> bool {
        if let Some(turn) = self.turn.as_ref() {
            turn.busy(holder);
        }
        self.body = Body::Payload(payload);
        self.busy = Some(holder.clone());
        !self.lease.expired(now)
    }
    /// Expiration is checked before entering the guest again, including when
    /// the group became free between the deadline and the next receiver turn.
    pub fn expired_busy(&self, now: Instant) -> Option<&mod_api::PayloadBusy> {
        if self.lease.expired(now) {
            self.busy.as_ref()
        } else {
            None
        }
    }
    /// The single final failure, after a Busy payload exhausted its original deadline.
    pub fn busy_failure(&self, holder: &mod_api::PayloadBusy) -> String {
        let diagnostic = format!(
            "payload {} deadline expired after {} s; last observed holder: {holder}",
            self.id.0,
            super::connection::PAYLOAD_DEADLINE.as_secs(),
        );
        #[cfg(any(test, feature = "test-payload-probe"))]
        super::staging_probe::failure(&self.id.0, &diagnostic);
        diagnostic
    }
    /// Owned transport buffers and reference storage, excluding this inline record.
    /// Work is conservatively charged its associated input per queued reference;
    /// arbitrary module-owned captures belong to the module, not a traversable packet.
    pub fn owned_heap_bytes(&self) -> usize {
        let body = match &self.body {
            Body::Payload(bytes) => bytes.0.capacity(),
            Body::Work(_) => self.lease.owned_bytes,
        };
        self.id
            .0
            .capacity()
            .saturating_add(body)
            .saturating_add(std::mem::size_of::<IngressLease>() + 2 * std::mem::size_of::<usize>())
    }
    pub fn new(
        transport: &Transport,
        phase: Phase,
        packet: CustomPayload,
    ) -> Result<Self, PayloadSendError> {
        match Self::classify(transport, phase, packet).map_err(|failure| failure.error)? {
            Classified::Application(incoming) => Ok(incoming),
            Classified::Raw(mut incoming) if incoming.len() == 1 => Ok(incoming.remove(0)),
            Classified::Raw(_) => Err(PayloadSendError::EncodeFailed(
                "a raw channel has several receivers".into(),
            )),
            Classified::Other(packet) => {
                Err(PayloadSendError::UnknownPayload(PayloadId(packet.channel)))
            }
        }
    }
    /// One successful receiving-slot lookup; ownership fallback is only on a miss.
    pub fn classify(
        transport: &Transport,
        phase: Phase,
        packet: CustomPayload,
    ) -> Result<Classified, Failure> {
        let Some(epoch) = transport.epoch() else {
            return if transport.local.owns(&packet.channel) {
                Err(Failure::new(
                    transport,
                    PayloadId(packet.channel),
                    PayloadSendError::NotNegotiated,
                ))
            } else {
                Ok(Classified::Other(packet))
            };
        };
        let fail = |error, packet: CustomPayload| {
            Failure::new(transport, PayloadId(packet.channel), error)
        };
        if !epoch.active() {
            return Err(fail(PayloadSendError::StaleEpoch, packet));
        }
        if epoch.phase != phase {
            return Err(fail(
                PayloadSendError::WrongProtocol(PayloadId(packet.channel.clone())),
                packet,
            ));
        }
        let receiver = match epoch.peer.receiver(phase, &packet.channel) {
            Ok(receiver) => receiver.clone(),
            Err(PayloadSendError::UnknownPayload(_)) => {
                return Self::raw(transport, &epoch, phase, packet);
            }
            Err(error) => return Err(fail(error, packet)),
        };
        let owned = packet
            .channel
            .capacity()
            .saturating_add(packet.data.capacity());
        let lease = transport
            .reserve_ingress(owned)
            .map_err(|error| Failure::new(transport, PayloadId(packet.channel.clone()), error))?;
        let main = epoch.main_task();
        Ok(Classified::Application(Self {
            epoch,
            id: PayloadId(packet.channel),
            body: Body::Payload(PayloadBytes(packet.data)),
            receiver,
            main,
            lease: Arc::new(lease),
            work_lease: None,
            raw: false,
            turn: None,
            busy: None,
        }))
    }
    /// A serverbound play message on a raw channel: one delivery for each
    /// declaring mod, in load order, each charged its own ingress credit. An
    /// undeclared or send-only channel stays an unrelated plugin channel.
    fn raw(
        transport: &Transport,
        epoch: &Arc<Epoch>,
        phase: Phase,
        packet: CustomPayload,
    ) -> Result<Classified, Failure> {
        let receivers = transport.local.raw_receivers(&packet.channel);
        if phase != Phase::Play || epoch.peer.side != super::Side::Server || receivers.is_empty() {
            return Ok(Classified::Other(packet));
        }
        let mut deliveries = Vec::with_capacity(receivers.len());
        for receiver in receivers {
            let owned = packet.channel.capacity().saturating_add(packet.data.len());
            let lease = transport.reserve_ingress(owned).map_err(|error| {
                Failure::new(transport, PayloadId(packet.channel.clone()), error)
            })?;
            deliveries.push(Self {
                epoch: epoch.clone(),
                id: PayloadId(packet.channel.clone()),
                body: Body::Payload(PayloadBytes(packet.data.clone())),
                receiver: receiver.clone(),
                main: epoch.main_task(),
                lease: Arc::new(lease),
                work_lease: None,
                raw: true,
                turn: None,
                busy: None,
            });
        }
        Ok(Classified::Raw(deliveries))
    }
    /// Reserve before cloning the channel or retaining the guest's work resource.
    pub fn work(
        &self,
        transport: &Transport,
        work: Arc<dyn PayloadWork>,
    ) -> Result<Self, PayloadContextError> {
        if !self.epoch.active() {
            return Err(PayloadContextError::Disconnected);
        }
        let lease = transport
            .reserve_ingress(self.id.0.len())
            .map_err(|error| match error {
                PayloadSendError::Backpressure => PayloadContextError::Backpressure,
                _ => PayloadContextError::Disconnected,
            })?;
        Ok(Self {
            epoch: self.epoch.clone(),
            id: self.id.clone(),
            receiver: self.receiver.clone(),
            main: self.epoch.main_task(),
            body: Body::Work(work),
            lease: self.lease.clone(),
            work_lease: Some(lease),
            raw: self.raw,
            turn: None,
            busy: None,
        })
    }
}
