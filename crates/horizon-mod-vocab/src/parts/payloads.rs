// exports: none (reached inside the crate only)
//! Load-owned raw payload registrations. No connection state lives here.
use super::*;
use crate::{
    HandlerThread, IPayloadHandler, PayloadDeclaration, PayloadDirection, PayloadId,
    PayloadProtocol, PayloadSendError, PayloadToken,
};

#[derive(Clone)]
pub(crate) struct Registered {
    pub(crate) declaration: PayloadDeclaration,
    pub(crate) owner: &'static str,
    pub(crate) owner_index: usize,
    pub(crate) server: Option<Arc<dyn IPayloadHandler>>,
    pub(crate) client: Option<Arc<dyn IPayloadHandler>>,
    pub(crate) client_thread: HandlerThread,
    /// Set once the server callback panicked: it is never invoked again, on any connection.
    pub(crate) server_panicked: Arc<std::sync::atomic::AtomicBool>,
    /// The same for the client callback.
    pub(crate) client_panicked: Arc<std::sync::atomic::AtomicBool>,
}

// no wire: frozen process-local registration storage contains callback trait objects and owning-mod identities.
/// Immutable payload registry for engine connection setup and dispatch.
/// It is absent entirely when no mod registers a payload.
#[derive(Clone, Default)]
#[non_exhaustive]
pub struct Payloads {
    pub(crate) phases: [HashMap<String, Registered>; 2],
    /// Raw play channels by exact id, each with its declaring mods in load order.
    pub(crate) raw: HashMap<String, Vec<RawRegistered>>,
    /// Typed declarations by the slot their tokens carry, in registration order.
    pub(crate) typed: Vec<TypedSlot>,
    /// The load these declarations belong to, never 0 (a refused registration's); tokens of
    /// another load are stale.
    pub(crate) load: u64,
}

/// What a typed token must match: its declaration as registered, before protocol splitting.
#[derive(Clone)]
pub(crate) struct TypedSlot {
    pub(crate) id: PayloadId,
    pub(crate) protocol: PayloadProtocol,
    pub(crate) direction: PayloadDirection,
    pub(crate) schema: u64,
    pub(crate) owner_index: usize,
}

#[derive(Clone)]
pub(crate) struct RawRegistered {
    pub(crate) owner: &'static str,
    pub(crate) owner_index: usize,
    pub(crate) server: Option<Arc<dyn IPayloadHandler>>,
    /// Set once the server callback panicked: it is never invoked again.
    pub(crate) panicked: Arc<std::sync::atomic::AtomicBool>,
}

// no wire: resolved process-local callback and its owning mod's live enable guard.
/// A receiving callback resolved once while a connection freezes its table.
#[derive(Clone)]
#[non_exhaustive]
pub struct PayloadReceiver {
    /// The sole registered receiver.
    handler: Arc<dyn IPayloadHandler>,
    owner: &'static str,
    /// Its chosen executor.
    pub thread: HandlerThread,
    owner_index: usize,
    /// Shared by every connection's receiver of this callback.
    panicked: Arc<std::sync::atomic::AtomicBool>,
}

impl PayloadReceiver {
    /// The receiving mod's id, for diagnostics.
    pub fn owner(&self) -> &'static str {
        self.owner
    }
    /// Recheck the owner without scanning registrations or resolving the id again: off when
    /// its mod is off or this callback panicked.
    pub fn enabled(&self) -> bool {
        crate::mod_index_on(self.owner_index)
            && !self.panicked.load(std::sync::atomic::Ordering::Acquire)
    }
    /// Invoke only this owner. The caller owns the root transaction and handles
    /// a native panic as `Failed`, never as retryable contention. A panic disables
    /// this callback for every connection; later calls refuse without running it.
    pub fn handle(
        &self,
        bytes: crate::PayloadBytes,
        context: &mut dyn crate::IPayloadContext,
    ) -> Result<(), crate::PayloadHandleError> {
        self.call("payload", || self.handler.handle(bytes, context))
            .map_err(crate::PayloadHandleError::Failed)?
    }
    /// Scheduled work retains its original receiving owner and enable guard.
    pub fn run_work(
        &self,
        work: &dyn crate::PayloadWork,
        context: &mut dyn crate::IPayloadContext,
    ) -> Result<(), String> {
        self.call("payload work", || work.run(context))?
    }
    fn call<T>(&self, operation: &'static str, call: impl FnOnce() -> T) -> Result<T, String> {
        if !self.enabled() {
            return Err("payload owner is disabled".into());
        }
        let _guard = crate::mark_mod(self.owner, operation);
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(call)).map_err(|_| {
            self.panicked
                .store(true, std::sync::atomic::Ordering::Release);
            "payload callback panicked; it is disabled".into()
        })
    }
}

impl Payloads {
    /// Iterate normalized single-protocol declarations for negotiation.
    pub fn declarations(&self) -> impl Iterator<Item = &PayloadDeclaration> {
        self.phases
            .iter()
            .flat_map(|phase| phase.values())
            .filter(|entry| crate::mod_index_on(entry.owner_index))
            .map(|entry| &entry.declaration)
    }
    /// Resolve the sole receiving owner. A disabled owner or absent callback has
    /// no handler; the connection must report failure, never silently skip it.
    pub fn handler(
        &self,
        id: &str,
        protocol: PayloadProtocol,
        direction: PayloadDirection,
    ) -> Option<PayloadReceiver> {
        let index = match protocol {
            PayloadProtocol::Configuration => 0,
            PayloadProtocol::Play => 1,
            PayloadProtocol::Common => return None,
        };
        let entry = self.phases[index].get(id)?;
        if !crate::mod_index_on(entry.owner_index) {
            return None;
        }
        match direction {
            PayloadDirection::ToClient => entry.client.clone().map(|handler| PayloadReceiver {
                handler,
                thread: entry.client_thread,
                owner_index: entry.owner_index,
                owner: entry.owner,
                panicked: entry.client_panicked.clone(),
            }),
            PayloadDirection::ToServer => entry.server.clone().map(|handler| PayloadReceiver {
                handler,
                thread: entry.declaration.thread,
                owner_index: entry.owner_index,
                owner: entry.owner,
                panicked: entry.server_panicked.clone(),
            }),
            PayloadDirection::Bidirectional => None,
        }
    }
    /// Whether `token` is one this load's registrar handed out: the declaration at its slot
    /// has its id, protocol, schema and a direction it sends in, and its owner is on.
    pub fn check_token(&self, token: &PayloadToken) -> Result<(), PayloadSendError> {
        let matches = token.load == self.load
            && token.direction != PayloadDirection::Bidirectional
            && usize::try_from(token.slot)
                .ok()
                .and_then(|slot| self.typed.get(slot))
                .is_some_and(|slot| {
                    slot.id == token.id
                        && slot.protocol == token.protocol
                        && slot.schema == token.schema
                        && (slot.direction == token.direction
                            || slot.direction == PayloadDirection::Bidirectional)
                        && crate::mod_index_on(slot.owner_index)
                });
        if matches {
            Ok(())
        } else {
            Err(PayloadSendError::StaleToken(token.id.clone()))
        }
    }
    /// The raw play channels an enabled mod declared, by exact id.
    pub fn raw_channels(&self) -> impl Iterator<Item = &str> {
        self.raw
            .iter()
            .filter(|(_, owners)| {
                owners
                    .iter()
                    .any(|owner| crate::mod_index_on(owner.owner_index))
            })
            .map(|(id, _)| id.as_str())
    }
    /// The enabled mods receiving raw channel `id` from clients, in load order,
    /// each on the main thread. Empty for an undeclared or send-only channel.
    pub fn raw_receivers(&self, id: &str) -> Vec<PayloadReceiver> {
        self.raw.get(id).map_or_else(Vec::new, |owners| {
            owners
                .iter()
                .filter(|owner| crate::mod_index_on(owner.owner_index))
                .filter_map(|owner| {
                    Some(PayloadReceiver {
                        handler: owner.server.clone()?,
                        owner: owner.owner,
                        thread: HandlerThread::Main,
                        owner_index: owner.owner_index,
                        panicked: owner.panicked.clone(),
                    })
                })
                .collect()
        })
    }
    /// Validate the client-only handler requirement before a client starts its
    /// connection. Dedicated servers do not require unused client callbacks.
    pub fn validate_client_handlers(&self) -> Result<(), Vec<String>> {
        let missing: Vec<_> = self
            .phases
            .iter()
            .flat_map(|phase| phase.values())
            .filter(|entry| {
                crate::mod_index_on(entry.owner_index)
                    && entry.declaration.direction != PayloadDirection::ToServer
                    && entry.client.is_none()
            })
            .map(|entry| {
                format!(
                    "{} has no client payload handler for {} in {:?}",
                    entry.owner, entry.declaration.id.0, entry.declaration.protocol
                )
            })
            .collect();
        if missing.is_empty() {
            Ok(())
        } else {
            Err(missing)
        }
    }
}

/// The frozen process registry, shared rather than copied into every connection.
// hidden: engine registry accessor exported only through __private::game.
#[doc(hidden)]
pub fn installed_payloads() -> Option<Arc<Payloads>> {
    installed().and_then(|installed| installed.keys.payloads.clone())
}

/// Check `token` against the installed load: what the host runs before every typed send.
/// With no payloads installed, every token is stale.
// hidden: engine send check exported only through __private::game.
#[doc(hidden)]
pub fn check_payload_token(token: &PayloadToken) -> Result<(), PayloadSendError> {
    installed_payloads().map_or_else(
        || Err(PayloadSendError::StaleToken(token.id.clone())),
        |payloads| payloads.check_token(token),
    )
}

/// Load-only operations backing the public payload registration views.
pub trait PayloadsRegistrar {
    /// Register a declaration and its receiving callbacks. Invalid declarations
    /// join the registrar's startup refusal list, naming the owning mod.
    fn add_payload(
        &mut self,
        declaration: PayloadDeclaration,
        server: Option<Arc<dyn IPayloadHandler>>,
        client: Option<Arc<dyn IPayloadHandler>>,
    );
    /// Attach the client callback to this owner's already declared clientbound
    /// entries in every protocol. Duplicates and unknown ids refuse startup.
    fn add_client_payload_handler(
        &mut self,
        id: &PayloadId,
        thread: HandlerThread,
        handler: Arc<dyn IPayloadHandler>,
    );
    /// Register a typed declaration: `declaration` carries the mod's own version, to which
    /// the registrar appends `schema`. Refused like [`add_payload`](Self::add_payload). The
    /// token names the declaration's slot and this load; a refused one names load 0, so every
    /// send with it is stale.
    fn add_typed_payload(
        &mut self,
        declaration: PayloadDeclaration,
        schema: u64,
        server: Option<Arc<dyn IPayloadHandler>>,
        client: Option<Arc<dyn IPayloadHandler>>,
    ) -> PayloadToken;
    /// Declare a raw play channel by exact id, with an optional server receiver.
    /// Reserved namespaces, a second declaration by the same mod and an id a
    /// typed payload uses refuse startup, naming the mods involved.
    fn add_raw_payload(&mut self, id: PayloadId, server: Option<Arc<dyn IPayloadHandler>>);
}
