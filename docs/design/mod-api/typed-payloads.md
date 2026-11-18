# Registered payloads and the typed-payload design

The registered **raw** transport implements [arismoko/horizon#2882](https://github.com/arismoko/horizon/issues/2882).
Typed registration and token binding over it are [below](#typed-registration-and-tokens-813-step-4b-1);
configuration snapshots remain work for [arismoko/horizon#813](https://github.com/arismoko/horizon/issues/813).
The transport contract is detailed in [Registered raw custom payload transport](../raw-custom-payload-transport.md).

## Register raw payloads once

Register during `Hooks::register`, in dependency order. `payload_handlers().registrar("1")`
returns a `PayloadRegistrar` view. Versions are exact, nonblank strings, not semver ranges.
Main-thread handling and required peer support are the defaults.
For a complete declaration value, `PayloadDeclaration::new` takes the channel,
version, protocol, direction, optional flag, and handler thread explicitly;
registration still owns channel and version validation.

A trait mod overrides `Hooks::register` directly; it does not need to implement a
block, item, or other Part. The example below assumes the owning mod's id is
`example`. Payload declarations use the same load-owned registrar and freeze point
as Parts, not a separate manifest channel list.

```rust,ignore
use mod_api::{
    Hooks, IPayloadContext, IPayloadHandler, PayloadBytes, PayloadHandleError, PayloadId, Registrar,
    RegistrationContext, horizon_mod,
};
use std::sync::Arc;

struct Update;
impl IPayloadHandler for Update {
    fn handle(
        &self,
        bytes: PayloadBytes,
        context: &mut dyn IPayloadContext,
    ) -> Result<(), PayloadHandleError> {
        // Decode this application's bytes here. Network handlers must enqueue
        // main-thread work before requesting a world Host.
        let _ = (bytes, context);
        Ok(())
    }
}
struct ExampleMod;

#[horizon_mod]
impl Hooks for ExampleMod {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        r.payload_handlers().registrar("1")
            .play_to_server(PayloadId("example:update".into()), Arc::new(Update));
    }
}
```

The registrar has `play_*`, `configuration_*`, and `common_*` methods for
`to_client`, `to_server`, and `bidirectional`. Common occupies both protocols.
A client handler may be supplied at registration or through
`RegisterClientPayloadHandlersEvent` before the installed table freezes.
`optional()`, `versioned(...)`, and `executes_on(HandlerThread::Network)` return
modified views; they do not modify the original view. Duplicate protocol/id
registrations, invalid ownership or versions, and missing receiving handlers are
refused. An id grants no authority merely because a caller constructs a `PayloadId`.

Native and wasm use the same declarations, opaque `PayloadBytes`, handlers, and
results. A callback receives the real connection's `IPayloadContext`, not a
broadcast event. Dispatch selects the one registered owner.

Handlers return `Result<(), PayloadHandleError>`. Application failures must use
`PayloadHandleError::Failed(message)` explicitly; `PayloadWork::run` still returns
`Result<(), String>`. Native panics and disabled owners are real failures, not busy
results. A wasm group that is occupied returns `Busy { payload, holder }` before
starting the callback, preserving the exact owned `PayloadBytes` and cheaply shared
holder names (`owner`, `export`, `thread`) plus `held_millis`.

The receiving transport, not the handler, retains busy bytes at the connection's
head so later input cannot overtake them. Retries retain original ingress credit,
queue budget and the original 60-second payload deadline; they do not renew the
deadline or consume another lease. Phase changes and disconnect cancel old work.
Nested cross-group calls refuse immediately rather than wait. Busy snapshots are
formatted only when a diagnostic is needed, not on each retry.

## Admission, sending, and context

Configuration exchanges declarations and freezes the compatible peer intersection.
A missing required declaration refuses admission. Matching protocol/id declarations
must agree on version and direction even when optional; mismatches refuse with the
id, reason and both declarations. Only one-sided optional absence permits admission,
and send then returns `UnsupportedPeer`
without a packet. Horizon-branded peers must also answer the global raw transport
revision mark on `horizon:payloads`, even with empty or all-optional declarations.
Query is `[0, 1]`; Offer starts `[1, 1]` followed by the record count. A missing
response or wrong revision refuses admission with a named raw-protocol cause. One
exception: a client meeting a Horizon server that sent nothing on `horizon:payloads`
(a build from before this negotiation) joins it like a vanilla server, with nothing
negotiated. This applies only to the connection's initial configuration; a
reconfiguration without a response still refuses. Once that initial configuration
finished with nothing received on `horizon:payloads`, payloads on channels the client
declared are dropped instead of refused: they are never decoded or delivered to a mod.
Such payloads arriving during configuration, before its finish, still refuse. A required
local declaration the server lacks still refuses admission.
This is not a build authentication mechanism. Non-Horizon vanilla peers retain
optional admission. Peers with no mod declarations exchange only protocol metadata and
an Offer of the engine's optional declarations (below); there is no per-tick scan of
loaded mods.

Clients send through `ClientHost::send_payload_to_server`; servers use
`PlayerRef::send_payload`. `IPayloadContext` offers `player`, `reply`, `disconnect`,
`enqueue_work`, and sided Host access. Configuration has no play player. A NETWORK
handler has no direct world access: enqueue work to obtain the receiving main-thread
context. Wrong-thread and wrong-side requests return explicit errors.

A client mod leaves the server or world with `host.disconnect(Some(reason))`, at the end of
the frame and outside every hook: the reason is shown on the disconnect screen, `None` leaves
as the pause menu's Disconnect does, a request belongs to the world it was made in, and
`DisconnectError::Unavailable` answers when no world is joined. (A server mod disconnects a
player with `player.disconnect`, documented with the player services.)

`Queued` means accepted locally, not acknowledged by the peer. Sending reserves
physical/provider credit before acceptance. Root commit publishes staged sends;
rollback publishes none. Replies enter a subsequent receiving turn, including
integrated play. Play delivery shares the connection's ordinary gameplay inbox, so
movement/actions before a raw message are applied before its MAIN handler, and
later inputs wait until that handler's root transaction finishes.

Work and prepared sends bind to a physical connection and protocol incarnation.
Reconfiguration invalidates old queued work; returning to Play cannot revive an old
prepared packet. Original input credit remains held through queued work and derived
actor effects, not merely until dequeue. Queue pressure returns `Backpressure` or
refuses the flooding session; there is no 32-message-per-tick rule or silent truncation.

## Capacity and interop

Registered application bytes use their own namespaced vanilla custom-payload channel.
There is **no application envelope**. Unknown unrelated plugin channels retain
vanilla's discard limits: 32,767 serverbound and 1,048,576 clientbound body bytes.
Those numbers do **not** cap a registered body.

A network frame must fit 2,097,151 encoded bytes. Compressed transport also limits
the complete uncompressed packet to 8,388,608 bytes. Both include packet/channel
framing; highly incompressible data may hit the encoded limit first. Private memory
connections do not inherit network framing caps. Resource quotas still apply and
are resource refusals, not a smaller codec body limit. Frame preparation is reused
for writing rather than recompressing solely for validation.

`ClientHost::send_packet` still supports unrelated vanilla/plugin interoperability.
It validates the packet and reserves outgoing credit, but publication waits for root
commit and the same physical protocol incarnation. It refuses every locally owned
registered channel and engine-reserved channel; it cannot bypass negotiation,
direction, or service authority. The former envelope hooks and tick-capped send
methods are removed, without aliases or fallback handlers. Private custom books
advertise their own version on `horizon:recipe_book_settings`; book support does not
depend on a mod transport flag.

## Raw plugin channels

A server mod can also speak a plain plugin channel that a vanilla client, a
client mod or the proxy uses (`bungeecord:main`, `voicechat:secret`), by
declaring it by its exact id: `registrar.play_raw(PayloadId(id), Some(handler))`,
or `None` for a channel the mod only sends on. A raw declaration is a server's
unnegotiated policy for that channel, not a payload: it is not in the
negotiation offer, makes no client mod-required, and a raw-only mod needs no
Horizon peer. `PlayerRef::send_payload` sends the exact bytes in a vanilla
custom payload to any client in play; above vanilla's clientbound limit
(1,048,576 bytes) it returns `PayloadTooLarge`. What a client sends on the
channel reaches every mod that declared it with a handler, once each, in mod
load order and on the main thread; one mod's error is logged and rolls back its
own work only. The serverbound limit stays vanilla's 32,767 bytes. An
undeclared channel is ignored as before. `minecraft:` and `horizon:` ids cannot
be declared, and a raw id that a typed payload also registers refuses load,
naming both mods. `PlayerRef::announced_channels` reads what the client
announced in `minecraft:register` (less `minecraft:unregister`), in
configuration and play: at most 128 channels of up to 64 bytes. Raw channels
are server-side only and take exact ids. Namespace wildcards are not supported.

A trait mod declares its raw channels in its own `Hooks::register`, through the
same registrar as typed payloads; no Part and no manifest entry are needed:

```rust,ignore
use mod_api::{
    Hooks, IPayloadContext, IPayloadHandler, PayloadBytes, PayloadHandleError, PayloadId, Registrar,
    RegistrationContext,
};
use std::sync::Arc;

struct VoiceSecret;
impl IPayloadHandler for VoiceSecret {
    fn handle(
        &self,
        bytes: PayloadBytes,
        context: &mut dyn IPayloadContext,
    ) -> Result<(), PayloadHandleError> {
        let player = context.player().map_err(|e| PayloadHandleError::Failed(format!("{e:?}")))?;
        let host = context.server_host().map_err(|e| PayloadHandleError::Failed(format!("{e:?}")))?;
        // Answer on the same channel; above 1,048,576 bytes this is PayloadTooLarge.
        let id = PayloadId("voicechat:secret".into());
        player.send_payload(host, &id, bytes)
            .map_err(|e| PayloadHandleError::Failed(format!("{e:?}")))?;
        Ok(())
    }
}

struct MyMod;
#[mod_api::horizon_mod]
impl Hooks for MyMod {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        r.payload_handlers()
            .registrar("1")
            .play_raw(PayloadId("voicechat:secret".into()), Some(Arc::new(VoiceSecret)))
            // Send-only: the mod sends on it but receives nothing.
            .play_raw(PayloadId("bungeecord:main".into()), None);
    }
}
```

## Stage 2: the value codec

`crates/horizon-mod-vocab/src/content/payload_codec.rs` holds the pure codec half of stage 2, exported from `mod_api`, and `#[derive(PayloadCodec)]` writes a type's codec from its fields. It is the part that touches neither registration nor delivery, so it stands apart from the raw transport of [arismoko/horizon#2882](https://github.com/arismoko/horizon/issues/2882). The typed handler adapter decodes a received body with it, and the token sends encode one.

| Item | Contract |
| --- | --- |
| `PayloadCodec` | The trait a payload's value type implements: `encode(&self, &mut PayloadWriter)`, `decode(&mut PayloadReader) -> Result<Self, PayloadDecodeError>` and `schema(visiting) -> u64`, its fingerprint. Both see only the buffer, never a `Host`. Implemented for `bool`, `u8`, `i8`, `i16`, `u16`, `i32`, `i64`, `f32`, `f64`, `VarInt`, `VarLong`, `String`, `Vec<T>`, `Option<T>` and `Box<T>` |
| Encoding | Vanilla's network encoding, so a ported payload keeps its exact bytes: big-endian numbers, VarInt and VarLong, a VarInt byte length before a string or byte array, a VarInt count before a list, one boolean byte before an optional value |
| `decode_payload::<T>(body)` | The whole body as one `T`; a byte left over is `Trailing { bytes }` |
| `encode_payload(&value, limit)` | The body, or `PayloadEncodeError`; the byte limit is the caller's (the transport's frame bound), checked before the buffer grows |
| `PAYLOAD_MAX_DEPTH` | 64. Each list, each present option and each `nested` level counts one; level 65 is `TooDeep` |
| `PAYLOAD_MAX_STRING_BYTES`, `PAYLOAD_MAX_LIST_ENTRIES` | 4096 each, the defaults of `String` and `Vec<T>`. A schema with another bound calls `str_within(limit)`, `bytes(limit)` or `list(limit, read)` itself, so these are not a transport ceiling |
| `PAYLOAD_DECODE_BUDGET` | 8,388,608 bytes of allocation for one value. A list is charged `count * size_of::<T>()` before it is reserved, an owned string its length, so a small body cannot ask for a large allocation |
| `#[derive(PayloadCodec)]` | The generated codec, in `crates/horizon-mod-api-macros/src/payload.rs`. A struct is its fields in declaration order with nothing between them. An enum is a VarInt tag, the variant's position from 0 whatever its discriminant, then that variant's fields; a tag no variant has is `InvalidDiscriminant`. Each field goes through its own `PayloadCodec`, so the bounds above hold inside a derived type, and it adds no nesting level of its own. An enum with no variants, a union and a field without a codec do not compile |
| `PayloadDecodeError` | `UnexpectedEnd`, `VarIntTooLong`, `NegativeLength`, `LengthOverLimit`, `Utf8`, `TooDeep`, `InvalidDiscriminant`, `NonFinite`, `BudgetExhausted`, `Trailing`, `InvalidValue`; has a Wire form |
| `PayloadEncodeError` | `TooLarge`, `LengthOverLimit`, `TooDeep`, `NonFinite`, `BudgetExhausted`; has a Wire form. An encoder refuses what the peer's decoder would refuse, its allocation budget included |

A length is read as a signed VarInt and checked for sign and bound before anything is reserved or copied; strings and byte arrays are borrowed from the body. A boolean other than 0 or 1 is refused (vanilla reads any non-zero byte as true). `finite_f32` and `finite_f64` are how a schema declares a finite float. No decode path indexes, and none panics on any input.

Two things this stage does not do. The bounded field kinds of [arismoko/horizon#811](https://github.com/arismoko/horizon/issues/811) (`DatapackFieldKind`) are not merged with this reader: they describe JSON entries the game decodes, are named for datapacks, and their issue is in flight; a shared name for the two belongs to whoever lands second. And the budget counts what the reader's own primitives allocate, not what a hand-written codec allocates beside them; native code has to obey the contract, as the section above says.


Strings use a 4096 UTF-8 byte default, not a character count. A schema may pass 3N bytes to `str_within` for a vanilla N-character limit. Each box, list and present option consumes one of 64 nesting levels; boxes charge their inner allocation size before decoding.

## Typed registration and tokens (arismoko/horizon#813 step 4b-1)

A typed payload is an ordinary raw declaration whose body is exactly `encode_payload(&value)`.
There is no second route and no envelope: the typed layer is registration, a token and two sends.

```rust,ignore
use mod_api::*;

#[derive(PayloadCodec)]
struct Counter {
    counter: VarInt,
}

static DOWN: std::sync::OnceLock<ToClient<Counter>> = std::sync::OnceLock::new();

fn register(r: &mut dyn Registrar) {
    let mut payloads = r.payload_handlers();
    let mut view = payloads.registrar("1");
    let _up: ToServer<Counter> = view.play_to_server_typed(
        PayloadId("example:counter".into()),
        |value: Counter, cx: &mut dyn IPayloadContext| {
            let reply = Counter { counter: VarInt(value.counter.0 + 1) };
            DOWN.get().unwrap().reply(cx, &reply).map(drop).map_err(|e| format!("{e:?}"))
        },
    );
    let down = view.play_to_client_typed(
        PayloadId("example:counter_reply".into()),
        |_: Counter, _: &mut dyn IPayloadContext| Ok(()),
    );
    let _ = DOWN.set(down);
}
```

| Item | Contract |
| --- | --- |
| Registration | `play_to_server_typed::<T>(id, handler) -> ToServer<T>`, `play_to_client_typed -> ToClient<T>`, `play_bidirectional_typed -> (ToServer<T>, ToClient<T>)`, and the `configuration_*` and `common_*` forms, on the same `PayloadRegistrar` view as the raw methods: its version, `optional()` and `executes_on(...)` apply. A typed value needs `PayloadCodec` only; it never crosses Wire, only its encoded body does. `T` is the only generic (`play_to_server_typed::<T>(id, handler)`). The handler is `Fn(T, &mut dyn IPayloadContext) -> Result<(), String>`; a typed clientbound handler is given at registration |
| Schema | `#[derive(PayloadCodec)]` writes `T`'s fingerprint: leaf kinds and bounds, field schemas in declaration order, variant positions. Field, variant and type names are not part of it; a type that holds itself has a finite one. A hand-written codec returns a leaf of its own name and version. The declaration's negotiated version is `<version>+payload-schema.<16 hex digits>`; a raw declaration's version cannot contain that marker |
| Admission | Another schema under one application version is `Mismatch::Schema`: refused, naming both versions, when either end requires the declaration; absent when both ends are optional, so a send returns `UnsupportedPeer`. Another application version stays a version mismatch, optional or not |
| Token | `ToServer<T>` and `ToClient<T>` carry a `PayloadToken`: the id, the declaration's slot in its load's typed table, the load, the protocol, the one direction it sends in and the schema of `T`. They have a Wire form, so a wasm mod keeps and passes them; a token remembers the schema of the type it is held as, computed once when it is registered or read as that type. A token grants nothing: the host checks it against the frozen table before every send (`Host::check_payload_token`, `IPayloadContext::check_payload_token`), and a forged, other-load or other-type token, or one whose owner is off, is `PayloadSendError::StaleToken` with nothing encoded or sent |
| Sending | `ToServer::send(host: &mut dyn ClientHost, &value)` goes through `ClientHost::send_payload_to_server`; `ToClient::send(host: &mut dyn ServerHost, player, &value)` through `PlayerRef::send_payload`; `reply(context, &value)` on either through `IPayloadContext::reply`. Each returns the raw send's results. The encoder refuses what the peer's decoder would refuse (`EncodeFailed` naming the id) before any bytes are queued, its allocation budget included: the writer charges `PAYLOAD_DECODE_BUDGET` by the reader's rules, with 64-bit sizes on every target. A wrong direction does not compile: a `ToServer` has no send to a player |
| Receiving | The adapter runs `decode_payload::<T>(body)`. A body that is not one `T` (a negative length, a count past its bound, nesting past 64, a trailing byte) fails the session by name, as a raw handler's error does, with no callback and nothing reserved before the bound is checked. A callback that panics is disabled for every connection, raw or typed: the transaction rolls back, no reply goes out, the session closes and a later message for it is refused without running it |
| Load refusals | Two owners of one id refuse load, naming the first. A typed id has one declaration: declaring it again in another protocol, typed or raw, refuses load, so a token's protocol is the only one its id exists in and a send in the other phase is `WrongProtocol`. The refused registration's token names load 0 and is always stale |
| Wasm | The same declarations, adapter and tokens: the guest decodes and runs its handler; the token crosses the Wire ABI as data and the host checks it as it does a native one |
| Cost | Registration and reading a token as a type walk the schema. A declared payload with no traffic makes no codec call; a send or reply checks its token with two `u64` compares and one table lookup, with no schema walk and no allocation before it encodes |

## Remaining typed work in arismoko/horizon#813

The earlier accepted design is amended by the raw transport's actual codec and
framing contracts above: its generic registered-body ceilings, typed application
envelope/numeric-slot proposal and 32-per-tick acceptance row are removed. The typed
layer above keeps registration and helpers outside object-safe Host dispatch, delegates
to the same raw transport and binds its token to the checked declaration, type,
protocol, direction and load.

Share schema/value codec primitives with arismoko/horizon#811. Wire is an ABI representation, not
permission to serialize Rust pointers, memory layouts, or `Any`. A typed value needs
`PayloadCodec` only: it never crosses Wire, only its encoded body and its token do
(the contract amendment on arismoko/horizon#813). Structured
codecs must check signed lengths, overflow, remaining bytes, cumulative allocation,
depth, discriminants, and registry references before allocating; require complete
consumption. Per-schema bounds must not silently reinstate a lower generic transport
ceiling. Native arbitrary allocation/process-aborting panics cannot be sandboxed.
Errors and wasm traps must abort the active transaction and discard staged sends.

Configuration snapshots still need revisioned atomic publication and acknowledgement
before admission, without overwriting local config files. They use this transport; they
get no separate networking implementation.

## Engine declarations: attachment sync (step 4a)

An engine service that sends on a registered channel declares it in the same negotiated
table as a mod (`raw_payloads::engine` in `horizon-client`). Every installation offers
its engine declarations, so a Horizon peer's Offer carries them even with no mods; each
is optional, so a peer without one is admitted and a send returns `UnsupportedPeer`
without a packet. The channel stays reserved: no mod may declare it, and the generic
packet API refuses it. With only engine declarations a client does not open negotiation
itself, it answers the server's Query, so a vanilla server is sent nothing new. A
negotiated incoming engine message goes to the engine's own decoder, not a mod callback;
an un-negotiated one, or one in the wrong direction or protocol, is a session refusal.

| Item | Contract |
| --- | --- |
| Declaration | `horizon:attachment_sync`, play, to-client, version `1`, optional |
| Recipients | Placed players whose play epoch has the channel negotiated, then the holder's trackers as before |
| Send | Prepared against the connection's current epoch, published at root commit after the player's held packets; rollback publishes none, a stale epoch publishes nothing |
| Format | Unchanged, byte for byte (holder, replace, changes) |
| Size | The connection's frame limits (arismoko/horizon#3087); the former 1,048,576-byte body cap is removed |
| Refusals | Past the frame limits or unencodable: the typed `PayloadSendError` logged once per connection epoch, message not sent, player stays joined. `Backpressure` closes the connection, as a full queue did before |
| Removed | The `brand == "horizon"` gate and the direct write to the connection |

Attachment sync keeps its message format. arismoko/horizon#796 stage 2 (tracked Part state) adds, on
top of this route, scoped incarnation, schema and committed revision, authorised tracking
snapshots, stale-update rejection, revision-gap handling and visibility-loss cleanup,
with arismoko/horizon#796's world/dimension, residency join, full-before-delta ordering, bounded assembly
and rollback proofs.

Acceptance for those future layers must exercise native and wasm over the real
integrated, LAN, and dedicated routes: exact typed decoding, mismatched/optional
peers, invalid tokens, malformed structured values, sender identity, ordered
application, next-turn replies, root rollback/traps, stale epochs, and resource
pressure with progress after drain. Measure wasm copying/allocation honestly; Wire
transfer is not zero-copy. Raw transport tests do not prove the unfinished
configuration-snapshot or attachment contracts.
