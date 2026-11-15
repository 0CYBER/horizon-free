//! Registration views over the single load-owned registrar.
use crate::{
    HandlerThread, IPayloadHandler, PayloadDeclaration, PayloadDirection, PayloadId,
    PayloadProtocol, Registrar,
};
use std::{cell::RefCell, rc::Rc, sync::Arc};

// no wire: this registration-only view borrows the current load registrar; declarations and callbacks cross separately.
/// NeoForge-style registration settings. Modifiers return another view and leave
/// this view unchanged. Register from Hooks::register; the installed table is frozen.
#[derive(Clone)]
#[non_exhaustive]
pub struct PayloadRegistrar<'a> {
    pub(super) registrar: Rc<RefCell<&'a mut dyn Registrar>>,
    pub(super) version: String,
    pub(super) optional: bool,
    pub(super) thread: HandlerThread,
}

impl<'a> PayloadRegistrar<'a> {
    pub(crate) fn new(registrar: &'a mut dyn Registrar, version: &str) -> Self {
        Self {
            registrar: Rc::new(RefCell::new(registrar)),
            version: version.into(),
            optional: false,
            thread: HandlerThread::Main,
        }
    }
    /// Use another exact version without changing the original view.
    pub fn versioned(&self, version: &str) -> Self {
        Self {
            version: version.into(),
            ..self.clone()
        }
    }
    /// Permit peers without these payloads, without changing the original view.
    pub fn optional(&self) -> Self {
        Self {
            optional: true,
            ..self.clone()
        }
    }
    /// Select the receiving executor without changing the original view.
    pub fn executes_on(&self, thread: HandlerThread) -> Self {
        Self {
            thread,
            ..self.clone()
        }
    }
    fn register(
        &mut self,
        id: PayloadId,
        protocol: PayloadProtocol,
        direction: PayloadDirection,
        server: Option<Arc<dyn IPayloadHandler>>,
        client: Option<Arc<dyn IPayloadHandler>>,
    ) -> &mut Self {
        self.registrar.borrow_mut().add_payload(
            PayloadDeclaration {
                id,
                version: self.version.clone(),
                protocol,
                direction,
                optional: self.optional,
                thread: self.thread,
            },
            server,
            client,
        );
        self
    }
}

impl PayloadRegistrar<'_> {
    /// Declare a raw play channel by its exact id (`bungeecord:main`,
    /// `voicechat:secret`): opaque bytes towards any peer, a vanilla client
    /// included, as a Paper plugin channel. It is not a negotiated payload:
    /// it is absent from the negotiation offer, makes no client mod-required
    /// and ignores this view's version and optional settings. The server sends
    /// on it with `PlayerRef::send_payload` (vanilla's 1,048,576-byte limit)
    /// and `server`, when given, receives what a client sends on it (vanilla's
    /// 32,767-byte limit), on the main thread. Several mods may declare one raw
    /// channel; each receives it once, in mod load order. `minecraft:` and
    /// `horizon:` ids, and an id a typed payload uses, refuse load.
    pub fn play_raw(
        &mut self,
        id: PayloadId,
        server: Option<Arc<dyn IPayloadHandler>>,
    ) -> &mut Self {
        self.registrar.borrow_mut().add_raw_payload(id, server);
        self
    }
}

macro_rules! methods {
    ($to_client:ident, $to_server:ident, $both:ident, $protocol:ident) => {
        impl PayloadRegistrar<'_> {
            /// Register a clientbound payload. Supply its handler here or through
            /// RegisterClientPayloadHandlersEvent before registration freezes.
            pub fn $to_client(
                &mut self,
                id: PayloadId,
                client: Option<Arc<dyn IPayloadHandler>>,
            ) -> &mut Self {
                self.register(
                    id,
                    PayloadProtocol::$protocol,
                    PayloadDirection::ToClient,
                    None,
                    client,
                )
            }
            /// Register a serverbound payload with its required receiving handler.
            pub fn $to_server(
                &mut self,
                id: PayloadId,
                server: Arc<dyn IPayloadHandler>,
            ) -> &mut Self {
                self.register(
                    id,
                    PayloadProtocol::$protocol,
                    PayloadDirection::ToServer,
                    Some(server),
                    None,
                )
            }
            /// Register both directions, with the required server handler and an
            /// optional client handler supplied here or by the client event.
            pub fn $both(
                &mut self,
                id: PayloadId,
                server: Arc<dyn IPayloadHandler>,
                client: Option<Arc<dyn IPayloadHandler>>,
            ) -> &mut Self {
                self.register(
                    id,
                    PayloadProtocol::$protocol,
                    PayloadDirection::Bidirectional,
                    Some(server),
                    client,
                )
            }
        }
    };
}
methods!(play_to_client, play_to_server, play_bidirectional, Play);
methods!(
    configuration_to_client,
    configuration_to_server,
    configuration_bidirectional,
    Configuration
);
methods!(
    common_to_client,
    common_to_server,
    common_bidirectional,
    Common
);

// no wire: the load event borrows its registrar and has no invocation data representation.
/// The load-time payload registration event, reached from the current Registrar.
#[non_exhaustive]
pub struct RegisterPayloadHandlersEvent<'a> {
    registrar: &'a mut dyn Registrar,
}
impl RegisterPayloadHandlersEvent<'_> {
    /// Start a required, main-thread registrar with this exact schema version.
    pub fn registrar(&mut self, version: &str) -> PayloadRegistrar<'_> {
        PayloadRegistrar::new(self.registrar, version)
    }
}

// no wire: the load event borrows its registrar; handler callbacks cross separately.
/// Assign client handlers to already declared clientbound payloads before freeze.
#[non_exhaustive]
pub struct RegisterClientPayloadHandlersEvent<'a> {
    registrar: &'a mut dyn Registrar,
}
impl RegisterClientPayloadHandlersEvent<'_> {
    /// Supply the default main-thread client handler for an existing declaration.
    pub fn register(&mut self, id: &PayloadId, handler: Arc<dyn IPayloadHandler>) {
        self.register_on(id, HandlerThread::Main, handler);
    }
    /// Supply a client handler and explicitly select its receiving executor.
    pub fn register_on(
        &mut self,
        id: &PayloadId,
        thread: HandlerThread,
        handler: Arc<dyn IPayloadHandler>,
    ) {
        self.registrar
            .add_client_payload_handler(id, thread, handler);
    }
}

impl dyn Registrar + '_ {
    /// View the once-per-load payload event during this mod's registration.
    pub fn payload_handlers(&mut self) -> RegisterPayloadHandlersEvent<'_> {
        RegisterPayloadHandlersEvent { registrar: self }
    }
    /// View the client-handler event after declaring this mod's payloads.
    pub fn client_payload_handlers(&mut self) -> RegisterClientPayloadHandlersEvent<'_> {
        RegisterClientPayloadHandlersEvent { registrar: self }
    }
}
