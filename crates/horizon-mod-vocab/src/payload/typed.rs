//! Typed payloads over the registered raw transport: registration that hands out a token bound
//! to the declaration it made, and sends that encode a value and go through the raw sends. The
//! body on the wire is exactly `encode_payload(&value)`; there is no envelope and no second route.
use super::{
    IPayloadContext, IPayloadHandler, PayloadAccepted, PayloadBytes, PayloadDeclaration,
    PayloadDirection, PayloadHandleError, PayloadId, PayloadProtocol, PayloadRegistrar,
    PayloadSendError,
};
use crate::wire::{Reader, Wire, WireError, Writer};
use crate::{
    ClientHost, PAYLOAD_DECODE_BUDGET, PayloadCodec, PlayerRef, ServerHost, decode_payload,
    encode_payload, payload_schema,
};
use std::{marker::PhantomData, sync::Arc};

/// What a typed registration claims: the declaration's slot in this load's frozen table, the
/// load, its protocol, the one direction the token sends in and its value type's schema
/// fingerprint. Constructing or decoding one grants nothing: every send checks it against the
/// frozen table and refuses a token the registrar did not hand out with `StaleToken`.
#[derive(Clone, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
// exhaustive: a claim is plain data; the frozen table, not its constructor, decides what it may send.
pub struct PayloadToken {
    /// The declared channel.
    pub id: PayloadId,
    /// The declaration's index in its load's typed table.
    pub slot: u32,
    /// The load that registered it; 0 for a registration that was refused.
    pub load: u64,
    /// The declared protocols.
    pub protocol: PayloadProtocol,
    /// The direction this token sends in: `ToServer` or `ToClient`, never `Bidirectional`.
    pub direction: PayloadDirection,
    /// The value type's schema fingerprint.
    pub schema: u64,
}

impl PayloadToken {
    /// A claim on the declaration at `slot` of load `load`. The registrar builds the ones it
    /// hands out; any other is refused at the first send.
    pub fn new(
        id: PayloadId,
        slot: u32,
        load: u64,
        protocol: PayloadProtocol,
        direction: PayloadDirection,
        schema: u64,
    ) -> Self {
        Self {
            id,
            slot,
            load,
            protocol,
            direction,
            schema,
        }
    }
    /// `Ok` when `value_schema`, the schema of the value type the token is held as, is the one
    /// it was handed out for and `direction` its own: two compares, no schema walk.
    fn claims(
        &self,
        value_schema: u64,
        direction: PayloadDirection,
    ) -> Result<(), PayloadSendError> {
        if self.direction == direction && self.schema == value_schema {
            Ok(())
        } else {
            Err(PayloadSendError::StaleToken(self.id.clone()))
        }
    }
    /// The checked body of `value`: what the peer's decoder accepts, or nothing is queued.
    fn body<T: PayloadCodec>(&self, value: &T) -> Result<PayloadBytes, PayloadSendError> {
        encode_payload(value, PAYLOAD_DECODE_BUDGET)
            .map(PayloadBytes)
            .map_err(|error| {
                PayloadSendError::EncodeFailed(format!("typed payload {}: {error:?}", self.id.0))
            })
    }
}

macro_rules! token {
    ($name:ident, $direction:ident, $doc:literal) => {
        #[doc = $doc]
        ///
        /// Keep it from registration and send with it; a wasm mod keeps and passes it like any
        /// Wire value. Each send checks the token against the frozen table before it encodes.
        // exhaustive: its fields are private; the registrar is the only constructor.
        // no wire: a manual codec below, the token's own; the value type is only a parameter.
        pub struct $name<T> {
            token: PayloadToken,
            /// `T`'s schema, walked once when the token is made or read as this type.
            value_schema: u64,
            value: PhantomData<fn() -> T>,
        }
        impl<T: PayloadCodec + 'static> $name<T> {
            fn new(token: PayloadToken) -> Self {
                Self {
                    token,
                    value_schema: payload_schema::<T>(),
                    value: PhantomData,
                }
            }
        }
        impl<T> $name<T> {
            /// The claim this token makes.
            pub fn token(&self) -> &PayloadToken {
                &self.token
            }
        }
        impl<T> Clone for $name<T> {
            fn clone(&self) -> Self {
                Self {
                    token: self.token.clone(),
                    value_schema: self.value_schema,
                    value: PhantomData,
                }
            }
        }
        impl<T> std::fmt::Debug for $name<T> {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_tuple(stringify!($name)).field(&self.token).finish()
            }
        }
        impl<T: PayloadCodec> $name<T> {
            /// Queue `value` as a reply on the connection `context` received on, as
            /// [`IPayloadContext::reply`] does, after the same token check as a send.
            pub fn reply(
                &self,
                context: &mut dyn IPayloadContext,
                value: &T,
            ) -> Result<PayloadAccepted, PayloadSendError> {
                self.token
                    .claims(self.value_schema, PayloadDirection::$direction)?;
                context.check_payload_token(&self.token)?;
                let body = self.token.body(value)?;
                context.reply(&self.token.id, body)
            }
        }
        /// The token's own claim; the value type is only its parameter. Reading one as `T`
        /// (the only way to retype a token) walks `T`'s schema once.
        impl<T: PayloadCodec + 'static> Wire for $name<T> {
            const LAYOUT_SEED: u64 = crate::wire::layout_mix(
                crate::wire::layout_name(stringify!($name)),
                <PayloadToken as Wire>::LAYOUT_SEED,
            );
            fn layout(visiting: &mut Vec<&'static str>) -> u64 {
                crate::wire::layout_mix(
                    crate::wire::layout_name(stringify!($name)),
                    <PayloadToken as Wire>::layout(visiting),
                )
            }
            const MIN: usize = <PayloadToken as Wire>::MIN;
            fn put(&self, out: &mut Writer) {
                self.token.put(out);
            }
            fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
                PayloadToken::take(input).map(Self::new)
            }
        }
    };
}
token!(
    ToServer,
    ToServer,
    "A typed serverbound payload: the token [`PayloadRegistrar`]'s `*_to_server_typed` and `*_bidirectional_typed` return."
);
token!(
    ToClient,
    ToClient,
    "A typed clientbound payload: the token [`PayloadRegistrar`]'s `*_to_client_typed` and `*_bidirectional_typed` return."
);

impl<T: PayloadCodec> ToServer<T> {
    /// Queue `value` to the connected server through
    /// [`ClientHost::send_payload_to_server`], with its results, after checking the token:
    /// `StaleToken` for one the registrar did not hand out for this type.
    pub fn send(
        &self,
        host: &mut dyn ClientHost,
        value: &T,
    ) -> Result<PayloadAccepted, PayloadSendError> {
        self.token
            .claims(self.value_schema, PayloadDirection::ToServer)?;
        host.check_payload_token(&self.token)?;
        let body = self.token.body(value)?;
        host.send_payload_to_server(&self.token.id, body)
    }
}

impl<T: PayloadCodec> ToClient<T> {
    /// Queue `value` to `player` through [`PlayerRef::send_payload`], with its results, after
    /// checking the token: `StaleToken` for one the registrar did not hand out for this type.
    ///
    /// A serverbound token has no `send` to a player, so the wrong direction does not compile:
    /// ```compile_fail
    /// # use mod_api::*;
    /// fn wrong(token: &ToServer<VarInt>, host: &mut dyn ServerHost, player: PlayerRef) {
    ///     let _ = token.send(host, player, &VarInt(7));
    /// }
    /// ```
    pub fn send(
        &self,
        host: &mut dyn ServerHost,
        player: PlayerRef,
        value: &T,
    ) -> Result<PayloadAccepted, PayloadSendError> {
        self.token
            .claims(self.value_schema, PayloadDirection::ToClient)?;
        host.check_payload_token(&self.token)?;
        let body = self.token.body(value)?;
        player.send_payload(host, &self.token.id, body)
    }
}

/// The receiving side of a typed declaration: decode the whole body as one `T`, then call the
/// mod's handler. A body that is not a `T` is the session's named refusal, and the handler does
/// not run.
struct Typed<T, F> {
    id: PayloadId,
    handler: F,
    value: PhantomData<fn() -> T>,
}

impl<T, F> IPayloadHandler for Typed<T, F>
where
    T: PayloadCodec + 'static,
    F: Fn(T, &mut dyn IPayloadContext) -> Result<(), String> + Send + Sync + 'static,
{
    fn handle(
        &self,
        bytes: PayloadBytes,
        context: &mut dyn IPayloadContext,
    ) -> Result<(), PayloadHandleError> {
        let value = decode_payload::<T>(&bytes.0).map_err(|error| {
            PayloadHandleError::Failed(format!(
                "typed payload {} refused its body: {error:?}",
                self.id.0
            ))
        })?;
        (self.handler)(value, context).map_err(PayloadHandleError::Failed)
    }
}

fn typed<T, F>(id: &PayloadId, handler: F) -> Arc<dyn IPayloadHandler>
where
    T: PayloadCodec + 'static,
    F: Fn(T, &mut dyn IPayloadContext) -> Result<(), String> + Send + Sync + 'static,
{
    Arc::new(Typed {
        id: id.clone(),
        handler,
        value: PhantomData::<fn() -> T>,
    })
}

impl PayloadRegistrar<'_> {
    fn register_typed<T: PayloadCodec + 'static>(
        &mut self,
        id: PayloadId,
        protocol: PayloadProtocol,
        direction: PayloadDirection,
        server: Option<Arc<dyn IPayloadHandler>>,
        client: Option<Arc<dyn IPayloadHandler>>,
    ) -> PayloadToken {
        let declaration = PayloadDeclaration {
            id,
            version: self.version.clone(),
            protocol,
            direction,
            optional: self.optional,
            thread: self.thread,
        };
        self.registrar.borrow_mut().add_typed_payload(
            declaration,
            payload_schema::<T>(),
            server,
            client,
        )
    }
}

fn sending(token: &PayloadToken, direction: PayloadDirection) -> PayloadToken {
    PayloadToken {
        direction,
        ..token.clone()
    }
}

macro_rules! typed_methods {
    ($to_client:ident, $to_server:ident, $both:ident, $protocol:ident) => {
        impl PayloadRegistrar<'_> {
            /// Register a typed clientbound payload with its client handler, which receives
            /// each body decoded as one `T`. The declaration is an ordinary raw one, with
            /// `T`'s schema fingerprint after this view's version.
            pub fn $to_client<T: PayloadCodec + 'static>(
                &mut self,
                id: PayloadId,
                client: impl Fn(T, &mut dyn IPayloadContext) -> Result<(), String>
                + Send
                + Sync
                + 'static,
            ) -> ToClient<T> {
                let client = typed(&id, client);
                ToClient::new(self.register_typed::<T>(
                    id,
                    PayloadProtocol::$protocol,
                    PayloadDirection::ToClient,
                    None,
                    Some(client),
                ))
            }
            /// Register a typed serverbound payload with its server handler, which receives
            /// each body decoded as one `T`, as the clientbound form does. `T` is the only
            /// generic, so it can be named:
            ///
            /// ```
            /// # use mod_api::*;
            /// fn register(r: &mut dyn Registrar) -> ToServer<VarInt> {
            ///     r.payload_handlers()
            ///         .registrar("1")
            ///         .play_to_server_typed::<VarInt>(
            ///             PayloadId("example:counter".into()),
            ///             |value, _| if value.0 >= 0 { Ok(()) } else { Err("negative".into()) },
            ///         )
            /// }
            /// ```
            pub fn $to_server<T: PayloadCodec + 'static>(
                &mut self,
                id: PayloadId,
                server: impl Fn(T, &mut dyn IPayloadContext) -> Result<(), String>
                + Send
                + Sync
                + 'static,
            ) -> ToServer<T> {
                let server = typed(&id, server);
                ToServer::new(self.register_typed::<T>(
                    id,
                    PayloadProtocol::$protocol,
                    PayloadDirection::ToServer,
                    Some(server),
                    None,
                ))
            }
            /// Register a typed payload in both directions, with both handlers: one
            /// declaration, and a token for each direction.
            pub fn $both<T: PayloadCodec + 'static>(
                &mut self,
                id: PayloadId,
                server: impl Fn(T, &mut dyn IPayloadContext) -> Result<(), String>
                + Send
                + Sync
                + 'static,
                client: impl Fn(T, &mut dyn IPayloadContext) -> Result<(), String>
                + Send
                + Sync
                + 'static,
            ) -> (ToServer<T>, ToClient<T>) {
                let server = typed(&id, server);
                let client = typed(&id, client);
                let token = self.register_typed::<T>(
                    id,
                    PayloadProtocol::$protocol,
                    PayloadDirection::Bidirectional,
                    Some(server),
                    Some(client),
                );
                (
                    ToServer::new(sending(&token, PayloadDirection::ToServer)),
                    ToClient::new(sending(&token, PayloadDirection::ToClient)),
                )
            }
        }
    };
}
typed_methods!(
    play_to_client_typed,
    play_to_server_typed,
    play_bidirectional_typed,
    Play
);
typed_methods!(
    configuration_to_client_typed,
    configuration_to_server_typed,
    configuration_bidirectional_typed,
    Configuration
);
typed_methods!(
    common_to_client_typed,
    common_to_server_typed,
    common_bidirectional_typed,
    Common
);
