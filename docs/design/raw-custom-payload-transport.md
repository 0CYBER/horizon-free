# Registered raw custom payload transport

This is the raw transport below the typed payload work in #813, implementing
#2882. The codec prerequisite is #2951. Names and contracts are based on
NeoForge 26.3's public payload API and the room's written clean-room specification;
no NeoForge implementation source was read by this implementation's author.

## One transport and one registration boundary

Replace the legacy envelope transport, its broadcast hooks, old send methods, brand gate,
30,000-byte envelope bound and 32-message counters outright. Application packets
use their declared namespaced custom-payload channel and exact opaque body bytes.
There is no Horizon application envelope and no second path for typed payloads.
Existing attachment synchronization is outside this raw transport migration.

Registration runs in dependency order during mod loading. `PayloadRegistrar`
provides play, configuration and common registrations, each ToClient, ToServer or
Bidirectional. `optional`, `versioned` and `executesOn` return modified registrar
views; required and main-thread handling are the defaults. Validate namespace
ownership, nonblank versions, duplicate protocol/id registrations and receiving
handlers once, then freeze. A common registration occupies both protocols.
Dispatch selects the one registered owner, not every loaded mod.

Native and wasm expose the same declarations, handlers and results. API aggregates
explicitly derive Wire; registration-only trait-object storage is not an API data
value. The raw body crosses Wire as a length followed by contiguous bytes.

## Connection admission and identity

Negotiate a bounded configuration manifest and compatible intersection. Matching
protocol/id declarations must agree on version and direction, even when both are
optional; a mismatch refuses admission with the id, reason and both declarations.
Only one-sided optional extras are absent from the agreed set. A missing required
declaration refuses admission. Optional means absence is permitted, not that an
incompatible matching declaration is ignored. A vanilla
peer is admitted when all local requirements are optional; sending an absent
optional payload returns UnsupportedPeer without a packet. There is no per-tick mod
work for empty registries. Configuration still exchanges a constant-size protocol
mark (and an empty Offer) for Horizon-branded peers, even with empty or all-optional
registries. On `horizon:payloads`, Query is bytes `[0, 1]`; Offer begins `[1, 1]`
then the manifest record count. The second byte is raw transport revision 1,
independent of application versions. A wrong revision or an absent marked response
from a Horizon-branded peer refuses admission with a named raw-protocol cause.
Servers send the Query before known packs so a compatible empty peer's Offer cannot
be overtaken by that barrier. Non-Horizon vanilla peers remain eligible for optional
admission; a brand is capability negotiation, not authentication of a build.

A connection owns its negotiated table, protocol and incarnation epoch. Application
messages never supply a player identity. A server handler resolves the authenticated
connection to its current PlayerRef; a client handler gets that connection's local
player. Queued work from an old epoch cannot affect a replacement connection.
Unnegotiated owned ids and malformed registered payloads fail the session with named
errors. Unrelated vanilla and plugin channels retain their existing behavior.
Generic packet sending cannot bypass the registered-channel checks.

## Limits are codec- and connection-aware

The unknown-id discard fallback retains vanilla's 32,767 serverbound and 1,048,576
clientbound body limits. These do **not** bound registered payloads. A frozen local
codec snapshot is supplied explicitly to packet readers and checked encoders; no
process-global or thread-local registration context selects the connection's codec.

Registered payloads have no generic body cap. A network packet, including its packet
id and payload channel framing, must fit `frame::MAX_FRAME` (2,097,151 bytes). With
compression enabled it must also fit `frame::MAX_UNCOMPRESSED` (8,388,608 bytes)
before compression, and its compressed contents must still fit MAX_FRAME. A memory
connection has no network framing cap. No automatic splitting is needed for Farmer's
Delight's four bodies, whose sizes are 0, 1, 2 and 8 bytes.

The mod send service returns a structured, named frame refusal before any bytes are
written. This deliberately differs from vanilla's encode-failure disconnect. Frame
preparation must be reused for writing, not recompressed just to validate it.

## Bounded delivery and transactions

A per-connection byte budget covers queued data and nonzero per-message bookkeeping,
including zero-byte messages. Locally saturated sends return Backpressure before
acceptance; there is no per-tick message count limit. Every actual ingress and
in-process transit queue must remain bounded, not only a downstream payload list.
Accepted means queued, not acknowledged; later session failure invalidates pending
work. Flood tests must demonstrate bounded retained memory and progress after drain.

Preserve order with ordinary gameplay ingress. Default handlers run on the logical
receiving main thread. IPayloadContext supplies player, reply, disconnect and
enqueueWork. Root transaction commit publishes sends; abort publishes none. Replies
cannot execute inline across the connection and are received on a subsequent turn.
Single-player and split-screen use their real connect_local sessions, not direct
callback shortcuts.

`IPayloadHandler::handle` returns `Result<(), PayloadHandleError>`, with explicit
`Failed(String)` for application and execution failures. Native panics and disabled
owners remain failures. `PayloadWork::run` retains its `Result<(), String>` contract.
Guest-group contention instead returns `Busy { payload, holder }` before the handler
starts, returning the same owned body and a resolved `PayloadBusy` snapshot (shared
owner/export/thread names and elapsed held milliseconds).

The connection transport must requeue busy input at its head, preserving both
payload and ordinary gameplay ordering, the original ingress lease and accounting,
and the original payload deadline. Existing queue byte budgets bound retained
memory. The deadline is 60 seconds, exceeding the largest guest execution budget;
retries never renew it. Deadline expiry reports the held mod/export/thread/duration.
Disconnect and protocol/epoch replacement cancel pending retries. Nested cross-group
calls refuse immediately rather than waiting. This is the caller's contract, not
queueing performed by the handler itself; it adds no detached retry worker.

Busy retries retain the existing 16 MiB physical ingress reservations across work
and main-thread moves. One connection-owned receiving turn spans MAIN root
publication or rollback. Pooled server receivers retry on their existing tick;
dedicated client and configuration receivers use timed condition-variable wakes,
so progress requires neither another network frame nor a detached worker.

## Acceptance and the amendment to #813

Replace #813's proposed 32-per-tick cap and its “33 sends” acceptance row: vanilla
has no such cap. The replacement checks byte-bounded pressure, explicit local
Backpressure, bounded flood memory and progress for more than 32 legal messages.
Its registered-body limit wording must distinguish discard limits from the actual
connection framing bounds above.

Exercise integrated single-player, split-screen, two remote players, LAN and
dedicated paths. Check sender attribution, ordering, next-turn replies, rollback,
epoch invalidation, optional vanilla admission, required mismatch, duplicate load
refusal, generic-send bypass and named decode failures. Native and wasm must both
exercise the real transport. Measure allocations for a 1 MiB wasm transfer; do not
claim zero-copy across Wire. The author supplies prepared-data self-check evidence;
review is static. The lane lead separately orders independent network test commands.

The `wasm-mods` fixture job runs
`game::payload_transport::wasm_config_tests::compiled_guest_runs_on_configuration_connection_thread`
by exact name and requires one passing test. It loads this run's compiled fixture,
negotiates payloads over TCP with the actual LAN connection, and holds known packs
and the serving Game thread while requiring two transformed, guest-counted replies.
This keeps dispatch in CONFIGURATION on `horizon-lan-conn`, before the Play I/O pool:
the existing guest-entry `check_current()` rejects a default-stack connection thread
instead of producing the expected replies. Its isolated child uses fresh game and
launcher homes and a scratch game directory/current working directory.
