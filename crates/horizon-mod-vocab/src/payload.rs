//! Registered raw custom-payload vocabulary shared by native and wasm mods.
#[path = "payload/registrar.rs"]
mod registrar;
pub use registrar::{
    PayloadRegistrar, RegisterClientPayloadHandlersEvent, RegisterPayloadHandlersEvent,
};

use crate::wire::{Reader, Wire, WireError, Writer};
use crate::{ClientHost, PlayerRef, ServerHost, Text};
use std::sync::Arc;

/// Opaque application bytes. The transport never prepends a mod envelope.
#[derive(Clone, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
// exhaustive: the single public field is the opaque-byte constructor contract.
pub struct PayloadBytes(pub Vec<u8>);

/// A namespaced application channel, validated by the registration boundary.
/// Sends resolve it against the frozen declarations; constructing one grants no authority.
#[derive(Clone, Debug, PartialEq, Eq, Hash, horizon_mod_api_macros::Wire)]
// exhaustive: the single public field is the channel constructor contract; registration owns validation.
pub struct PayloadId(pub String);

/// The protocols occupied by a registration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
// exhaustive: the closed registration protocol set is configuration, play, or their union.
pub enum PayloadProtocol {
    /// Configuration only.
    Configuration,
    /// Play only.
    Play,
    /// Both configuration and play.
    Common,
}

/// The declared flow; bidirectional declarations must agree on both peers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
// exhaustive: the closed registration flow set is clientbound, serverbound, or both.
pub enum PayloadDirection {
    /// Server to client.
    ToClient,
    /// Client to server.
    ToServer,
    /// Both directions.
    Bidirectional,
}

/// Where a receiving handler executes. Network handlers must enqueue world work.
#[derive(Clone, Copy, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
// exhaustive: the two executors define the complete world-access scheduling contract.
pub enum HandlerThread {
    /// The logical receiving main thread (the default).
    Main,
    /// The connection's network executor, without direct world access.
    Network,
}

/// A registration's complete compatibility and scheduling declaration.
#[derive(Clone, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
// exhaustive: callers construct the complete compatibility and scheduling record explicitly.
pub struct PayloadDeclaration {
    /// Exact channel written into vanilla custom-payload framing.
    pub id: PayloadId,
    /// Exact, nonblank schema version; no semver ranges.
    pub version: String,
    /// Protocols in which the channel exists.
    pub protocol: PayloadProtocol,
    /// Receiving directions.
    pub direction: PayloadDirection,
    /// Whether an absent peer declaration permits admission.
    pub optional: bool,
    /// The receiving executor.
    pub thread: HandlerThread,
}

impl PayloadDeclaration {
    /// Constructs a complete declaration with explicit compatibility and scheduling choices.
    /// The registration boundary validates the channel and exact, nonblank version.
    pub fn new(
        id: PayloadId,
        version: String,
        protocol: PayloadProtocol,
        direction: PayloadDirection,
        optional: bool,
        thread: HandlerThread,
    ) -> Self {
        Self {
            id,
            version,
            protocol,
            direction,
            optional,
            thread,
        }
    }
}

/// Queued locally, not a delivery acknowledgement. Disconnect invalidates pending work.
#[derive(Clone, Copy, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
// exhaustive: local queue acceptance is the only success state, never a remote acknowledgement.
pub enum PayloadAccepted {
    /// Accepted by the current connection and transaction.
    Queued,
}

/// Which whole-packet framing bound refused a send.
#[derive(Clone, Copy, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
// exhaustive: physical framing has exactly encoded and pre-compression bounds.
pub enum PayloadFrameStage {
    /// Contents after the optional compression header and compression.
    Encoded,
    /// Packet bytes before compression, including packet id and channel framing.
    Uncompressed,
}

/// Explicit local refusal. No rejected send writes any application bytes.
#[derive(Clone, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
// exhaustive: callers can handle every explicit local refusal in the closed Wire result schema.
pub enum PayloadSendError {
    /// No local declaration has this id.
    UnknownPayload(PayloadId),
    /// The optional channel is absent from the compatible peer intersection.
    UnsupportedPeer(PayloadId),
    /// The declaration does not permit this sending direction.
    WrongDirection(PayloadId),
    /// The declaration is not present in the connection's current protocol.
    WrongProtocol(PayloadId),
    /// There is no active connection.
    Disconnected,
    /// The configuration exchange has not frozen a peer table yet.
    NotNegotiated,
    /// Work belongs to a replaced connection incarnation.
    StaleEpoch,
    /// The connection's byte budget cannot accept the message now.
    Backpressure,
    /// Whole-packet framing refused the send, not an arbitrary raw-body cap.
    FrameTooLarge {
        /// Application channel.
        id: PayloadId,
        /// Refusing stage.
        stage: PayloadFrameStage,
        /// Whole-packet size at that stage.
        size: u64,
        /// Inclusive bound at that stage.
        limit: u64,
    },
    /// A connection codec could not prepare the packet.
    EncodeFailed(String),
    /// A raw channel's body is above vanilla's custom-payload limit for this
    /// direction (`ClientboundCustomPayloadPacket.MAX_PAYLOAD_SIZE`, 1,048,576 bytes).
    PayloadTooLarge {
        /// Raw channel.
        id: PayloadId,
        /// Body size in bytes, excluding the channel.
        size: u64,
        /// Inclusive vanilla limit.
        limit: u64,
    },
}

impl crate::NoBackend for PayloadSendError {
    fn no_backend() -> Self {
        Self::Disconnected
    }
}

/// A context operation cannot run in this phase, side or executor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
// exhaustive: the closed Wire result schema enumerates all context capability refusals.
pub enum PayloadContextError {
    /// Configuration has no authenticated play player.
    NoPlayer,
    /// The requested host is on the opposite logical side.
    WrongSide,
    /// World access requires enqueueWork on the main thread.
    WrongThread,
    /// The connection incarnation has ended.
    Disconnected,
    /// No space remains for queued work.
    Backpressure,
}

/// Status of work submitted through the payload context.
#[derive(Clone, Copy, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
// exhaustive: work either completes in the current root or queues on the receiving main thread.
pub enum PayloadWorkStatus {
    /// A main-thread submission ran immediately in the current transaction.
    Complete,
    /// A network-thread submission is scheduled on the receiving main thread.
    Queued,
}

/// A cheap snapshot of the guest currently occupying the receiving group.
/// Names share the resolved holder metadata; taking a snapshot does not format diagnostics.
#[derive(Clone, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
// exhaustive: the closed diagnostic snapshot identifies the holder and its elapsed occupancy.
pub struct PayloadBusy {
    /// The holding mod's id.
    pub owner: Arc<str>,
    /// The guest export holding the group.
    pub export: Arc<str>,
    /// The holding thread's diagnostic identity.
    pub thread: Arc<str>,
    /// Elapsed occupancy when the contention was observed.
    pub held_millis: u64,
}

impl std::fmt::Display for PayloadBusy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "mod {} export {} on thread {} held for {} ms",
            self.owner, self.export, self.thread, self.held_millis
        )
    }
}

/// A receiving callback's closed failure or contention result.
///
/// `Busy` is not an application failure: the handler did not start and returns the
/// same owned bytes. The transport, not the handler, must retain them at the head
/// of the receiving connection's queue, preserving order and original ingress
/// accounting. Retry is bounded by that queue's budget and the payload's original
/// deadline, and cancelled when its connection phase or epoch ends. A nested
/// cross-group call refuses immediately rather than waiting.
#[derive(Clone, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
// exhaustive: callers distinguish real callback failure from unstarted, retryable contention.
pub enum PayloadHandleError {
    /// Application decoding, callback execution, or owner availability failed.
    Failed(String),
    /// The callback did not start; retain these exact bytes for an ordered retry.
    Busy {
        /// The original owned application buffer, never a retry copy.
        payload: PayloadBytes,
        /// The resolved holder at the time contention was observed.
        holder: PayloadBusy,
    },
}

crate::wire::wire_handlers::handler!(dyn IPayloadHandler, "IPayloadHandler");
crate::wire::wire_handlers::handler!(dyn PayloadWork, "PayloadWork");

/// One receiving owner for a registered id and direction, never an all-mod broadcast.
/// Return `Failed` when application decoding fails; the session names the channel
/// and fails instead of silently dropping the malformed application message.
/// `Busy` returns unconsumed bytes to the transport under [`PayloadHandleError`]'s
/// ordering and deadline contract; this callback does not itself queue a retry.
pub trait IPayloadHandler: Send + Sync + 'static {
    /// Handle exact application bytes using connection-derived context.
    fn handle(
        &self,
        payload: PayloadBytes,
        context: &mut dyn IPayloadContext,
    ) -> Result<(), PayloadHandleError>;
}

/// Work scheduled on the logical receiving main thread.
pub trait PayloadWork: Send + Sync + 'static {
    /// Execute with the same connection-derived context and transaction rules.
    fn run(&self, context: &mut dyn IPayloadContext) -> Result<(), String>;
}

/// The receiving connection's context, not a player chosen from application bytes.
/// World hosts are available only on the logical main thread. Network handlers
/// use enqueue_work; main-thread submissions execute immediately.
pub trait IPayloadContext {
    /// The sending server player or receiving local player, only in play.
    fn player(&self) -> Result<PlayerRef, PayloadContextError>;
    /// The single receiving direction, never Bidirectional.
    fn flow(&self) -> PayloadDirection;
    /// The current protocol, never Common.
    fn protocol(&self) -> PayloadProtocol;
    /// The server host on the server main thread.
    fn server_host(&mut self) -> Result<&mut dyn ServerHost, PayloadContextError>;
    /// The client host on the client main thread.
    fn client_host(&mut self) -> Result<&mut dyn ClientHost, PayloadContextError>;
    /// Queue a reply on this same connection. It cannot execute inline at the peer.
    fn reply(
        &mut self,
        id: &PayloadId,
        bytes: PayloadBytes,
    ) -> Result<PayloadAccepted, PayloadSendError>;
    /// Disconnect this session with the supplied component.
    fn disconnect(&mut self, reason: Text);
    /// Run now on the main thread or schedule there from the network executor.
    /// Failure of the work is reported as a named session failure.
    fn enqueue_work(
        &mut self,
        work: Arc<dyn PayloadWork>,
    ) -> Result<PayloadWorkStatus, PayloadContextError>;
}
