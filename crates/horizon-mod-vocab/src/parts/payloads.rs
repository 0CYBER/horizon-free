// exports: none (reached inside the crate only)
//! Load-owned raw payload registrations. No connection state lives here.
use super::*;
use crate::{
    HandlerThread, IPayloadHandler, PayloadDeclaration, PayloadDirection, PayloadId,
    PayloadProtocol,
};

#[derive(Clone)]
pub(crate) struct Registered {
    pub(crate) declaration: PayloadDeclaration,
    pub(crate) owner: &'static str,
    pub(crate) owner_index: usize,
    pub(crate) server: Option<Arc<dyn IPayloadHandler>>,
    pub(crate) client: Option<Arc<dyn IPayloadHandler>>,
    pub(crate) client_thread: HandlerThread,
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
}

#[derive(Clone)]
pub(crate) struct RawRegistered {
    pub(crate) owner: &'static str,
    pub(crate) owner_index: usize,
    pub(crate) server: Option<Arc<dyn IPayloadHandler>>,
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
}

impl PayloadReceiver {
    /// The receiving mod's id, for diagnostics.
    pub fn owner(&self) -> &'static str {
        self.owner
    }
    /// Recheck the owner without scanning registrations or resolving the id again.
    pub fn enabled(&self) -> bool {
        crate::mod_index_on(self.owner_index)
    }
    /// Invoke only this owner. The caller owns the root transaction and handles
    /// a native panic as `Failed`, never as retryable contention.
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
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(call))
            .map_err(|_| "payload callback panicked".into())
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
            }),
            PayloadDirection::ToServer => entry.server.clone().map(|handler| PayloadReceiver {
                handler,
                thread: entry.declaration.thread,
                owner_index: entry.owner_index,
                owner: entry.owner,
            }),
            PayloadDirection::Bidirectional => None,
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
    /// Declare a raw play channel by exact id, with an optional server receiver.
    /// Reserved namespaces, a second declaration by the same mod and an id a
    /// typed payload uses refuse startup, naming the mods involved.
    fn add_raw_payload(&mut self, id: PayloadId, server: Option<Arc<dyn IPayloadHandler>>);
}
