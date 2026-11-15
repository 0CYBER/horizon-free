# Design: offline players and selection

Provisional answer to [arismoko/horizon#989](https://github.com/arismoko/horizon/issues/989), including Ari’s release comment: move the existing user cache into each authoritative server Game lifecycle; extract one immutable selector evaluator with an explicit ordering policy. Read-only random selection uses a required caller seed and a temporary RNG, never gameplay RNG. This is an unimplemented design with [provisional maintainer decisions](https://github.com/arismoko/horizon/issues/989#issuecomment-6733678791) on persistence, randomness and paging. No stages have been filed.

## Evidence and limits

Vanilla 26.3 queries: `python3 tools/rea_vanilla.py method net.minecraft.commands.arguments.selector.EntitySelector sortAndLimit` confirms ordering precedes limiting and runs only with more than one result. `python3 tools/rea_vanilla.py method net.minecraft.server.players.CachedUserNameToIdResolver save` confirms persistence of the top 1000 MRU profiles and ignored IOExceptions. The attempted EntitySelectorOptions `<clinit>` query failed; exact vanilla shuffle RNG/draw trace remains unverified. Vanilla client Services is created with the game directory; its client cache is scoped to that directory. Do not describe seeded service results as vanilla random-command parity.

The issue explicitly names no NeoForge counterpart. The public [26.3 Javadoc index](https://aldak.netlify.app/javadoc/26.3.x/) was reachable and identifies 26.3-26.3.0.1-beta; it does not establish an offline enumeration or immutable random-selection contract. Those NeoForge capabilities and limits remain unverified; require public documentation before adopting any counterpart claim. No NeoForge source, patches or other project source was read.

## Verified owners and routes

Paths and lines are from the inspected worktree base; proposed new ownership below is not present yet.

| Route | Existing owner and proposed treatment |
| --- | --- |
| Cache load, dedicated | `crates/horizon-client/src/game/interface/dedicated/mod.rs:943` loads UserCache, :949 installs the static declared at :171. Transfer that same instance into server-owned state; remove the static and redirect all consumers. |
| Join | `crates/horizon-client/src/game/network/lan/mod.rs:2646` calls dedicated::player_joined, implemented at `game/interface/dedicated/mod.rs:174`. Move admission recording to the shared successful join boundary for dedicated, integrated owner and LAN guests; failed authentication is excluded. |
| Existing lookup consumers | `game/interface/dedicated/mod.rs:184` cached_user_by_id and :1625 name resolution must use the same instance. Reads must not call mutating get_by_id/get_by_name. |
| Cache storage | `crates/horizon-server-files/src/user_cache.rs:38` UserCache, :53 load, :121 add, :136 get_by_id, :145 find_by_name, :210 serialization, :227 save. Add immutable enumeration over authoritative UUID entries here, without parsing JSON or exposing internal records. |
| Integrated creation / shutdown | `game/world/integrated.rs:849` supplies owner profile, :898 stop_integrated, :949 stop_serving. Construct cache for the serving Game, not the window or remote client. LAN publication shares this owner. |
| Dedicated shutdown | `game/interface/dedicated/mod.rs:2376` stop_dedicated must flush/drop its owned cache, including failure cleanup. |
| Server identity | `game/modding/native_lifecycle.rs:66` NativeLifecycle, :85 epoch, :124 begin_close, :131 close failure, :144 close final. Bind service access to this existing identity; no second generation counter. |
| Selector | `game/administration/commands/mod.rs:795` select, :835 level switching, :855 random ordering, :870 select_in_level, :1141 command-level switching. Extract filtering and ordering once; commands and service both call it. |
| Typed syntax | `crates/horizon-command/src/args.rs:334` Sort and :345 Selector, plus `selector.rs:99` parser. Keep one parser; public records are API-owned conversions, not parser internals. |

The game paths in the table abbreviate `crates/horizon-client/src/`. Remote windows and LAN guests have no authoritative service; server commands, integrated owner, network guests and dedicated console ultimately use the same server evaluator. Command execution may still switch levels to apply actions after selection; immutable evaluation itself must not.

## Per-server records

Expose offline_players through the focused object-safe server player service using `&dyn ServerHost`. Return owned API UUID/name records, never PlayerRef or save paths. “Known offline” means a current, nonexpired authoritative cache UUID record absent from the server-wide connected-player set, including profiles learned by successful name resolution; it does not mean every UUID with a playerdata file or every account ever seen. Deduplicate by UUID, use the latest authoritative name, order by UUID, and evaluate expiration against the host call’s captured time without eviction or MRU writes. A disconnected player becomes visible immediately; reconnect excludes them immediately.

Each serving Game owns exactly one existing UserCache. Dedicated keeps its configured universe cache location. The provisional decision is world-local usercache.json for integrated (singleplayer) worlds, selected by server startup configuration; LAN shares it. This is a deliberate scope choice, not vanilla placement: vanilla’s client cache is scoped to the game directory. Only UserCache loads/saves it. No service file reader, directory scan, global fallback or profile-network lookup. Separate server roots isolate records; refuse concurrent writable ownership of the same cache path rather than permitting racing writers. Explicitly sharing a persisted root across sequential lifetimes intentionally restores its records.

Initialize before ready, record the owner on successful admission, flush through the existing persistence path on orderly shutdown, and drop state on failed open or close as well. Retire access through NativeLifecycle; a retained service token cannot read the next world. Report persistence failures through server diagnostics without claiming durable success from this read API. Preserve existing cache retention rather than imposing the 1000 serialized-entry limit on in-memory enumeration.

## One immutable evaluator and explicit randomness

Expose select_entities on a focused object-safe server selection service. API-owned SelectorSpec and SelectionSource records with explicit `derive(Wire)` contain supported filters, sort, limit, source level, finite position/rotation and optional existing EntityRef. Resolve source identity through existing lifecycle/entity holders; reject stale, foreign-server or wrong-level source handles. No implicit mutable command source. Console @s returns empty. Relative positions use the supplied source; spatial constraints restrict the source dimension, while unconstrained queries retain canonical loaded-dimension traversal. Never force-load chunks or levels.

Extract a borrowed SelectionView over active and parked levels and all connected players. Move every existing predicate, candidate-order rule, distance calculation and tie rule into one evaluator over that view. Audit NBT, scoreboard and advancement predicates for hidden mutation. Unsupported predicates return UnsupportedSelectorCapability before evaluating, with identical native/wasm support; never silently skip a filter. Existing commands parse into this same evaluator. No second selector, cloned Game, swap-to-read, RefCell or mutex-backed mutation.

Separate evaluation from ordering entropy. The command adapter supplies its existing gameplay RNG explicitly to the common ordering routine and preserves command draw order. The provisional decision requires caller-seeded reads: encode ordering as `Random { seed: u64 }`, making a missing seed unrepresentable after decoding. Only the seed crosses `Wire`, never an RNG object. Initialize a local Java-compatible RNG from the seed, use the same descending Fisher–Yates routine and bounded draws, then limit. Before claiming parity, add an oracle fixture against vanilla’s `Collections.shuffle`, since Horizon currently consumes world RNG. Pin independent command and seeded-read fixtures before extraction ships; preserving today’s command RNG trace does not certify vanilla parity. Zero or one candidate takes zero draws. Nothing is stored between calls. Same seed, candidate order and state yields the same result; changing a seed need not change the result. Repeated reads never advance world/entity RNG, a global query counter, MRU state or wall-clock entropy. This is deliberate reproducible sampling, not a promise of fresh randomness per call. A caller wanting fresh samples chooses new seeds outside this service.

Return live EntityRefs through existing entity holders, including parked-level identities; do not invent query-specific generations or publish provisional residency. Revalidation on later use is mandatory. Reads see the current authoritative action view but emit no journal events; outer rollback invalidates any tentative handles through existing rules. If extraction requires lifting a pass, retirement and journal emission remain deferred to root commit. These services add no save operation; existing saves must remain refused while an action savepoint is open.

## Transport, bounds and failures

Generate native and wasm service glue and Wire records from the catalog together; no target-specific helpers or compatibility aliases. Both targets validate once at the input boundary, including encoded sizes and predicate counts before parsing or allocation: finite coordinates/rotation/distances, ordered nonnegative ranges, valid selector kinds, positive limits, bounded strings and predicate counts. Recommend 256 predicates, 32 KiB encoded spec and 4096 records per response page, with bounded continuation tied to the existing lifecycle and view revision; changed views return SelectionChanged rather than mixing pages. Pages must preserve complete vanilla candidate capacity: never cap the server population or silently truncate results. Capacity-preserving paging is the provisional decision: service byte budgets must never silently reduce what vanilla commands can select. Numerical limits remain proposed pending transport feasibility checks. Account for names, handles, envelope overhead and temporary sort buffers in byte budgets, not just record counts.

No mod callback is introduced, so there is no new callback registration. If later registration exposes predicate providers, refuse it as UnsupportedSelectorCapability until a documented read-only native/wasm contract exists. Native unwind or wasm trap at the existing host boundary returns failure and no partial response; release borrows and temporary buffers, perform no selector retry or gameplay mutation, and retain normal runtime diagnostics. Do not claim panic=abort recovery.

No subscriptions, per-tick cache rebuilds, all-mod scans or query allocations when unused: the added query cost is O(1) when uncalled. Existing admission cache work remains at admission; explicit queries scan matching authoritative records/entities and allocate only bounded response/work buffers. No-listener command behavior and gameplay RNG trace remain unchanged.

## Acceptance matrix

These are proposed implementation fixtures, not tests executed for this note.

| Fixture | Expected result |
| --- | --- |
| UUID 1/Alex joins then disconnects, unexpired | Exactly [(1, Alex)] on integrated, LAN host and dedicated; reconnect yields []. |
| Cached UUID 2/Steve learned by resolution, no join | Included while offline; expired UUID 3 excluded with zero MRU/cache writes. |
| Empty world / console @s | [] / []; zero random draws. |
| Source (10,64,10), entities at (12,64,10) and (30,64,10), radius 3 | Exactly first entity; source offset +2 included, +20 excluded. |
| Seed 7, unchanged ordered candidates, two reads | Identical ordered handles; gameplay RNG state before = after both reads. Next command random result equals baseline without reads. |
| Seeded random over [A,B,C], limit 2 | Two distinct members; independent vanilla `Collections.shuffle` oracle pins exact permutation before extraction ships; separate command fixture pins its entropy/draw trace. |
| Active and parked dimensions | Unbounded query sees both; radius query sees source dimension only; no level swaps or loads. |
| Server A root / server B different root | A disconnects UUID 1; B returns []; old A token is stale after shutdown, including failed shutdown. |
| Invalid/trapped request | NaN, limit 0, oversized spec and foreign source each fail before traversal; native unwind/wasm trap yields no partial results and unchanged RNG. |
| 5000 records | With sufficient byte budget and proposed page sizes, pages 4096 and 904, no loss; changed lifecycle or revision refuses continuation. No 1000-entry in-memory cap. |
| Integrated reopen / different world | Same world restores records; another world is isolated; LAN shares its host cache. Verify complete Services handoff. |
| Random ordering Wire round trip | `Random { seed: u64 }` retains seed; missing seed cannot decode; no RNG object crosses Wire. |
| Large names/handles and vanilla-capacity selection | Budget names, handles, envelope and temporary sort buffers; preserve all results and command capacity. |
| Predicate audit / malformed encoded input | Transitive predicates remain immutable; validate encoded sizes and predicate counts once before parsing/allocation on native and wasm. |
| No service calls | Zero new per-tick allocations/scans; command selections and RNG match prior behavior. |

## Proposed stages

File none. First establish remaining public/vanilla evidence and transport feasibility under the recorded provisional decisions. Then relocate cache ownership and all lookup/admission/shutdown routes. Extract the shared immutable evaluator and verify command parity before adding services. Finally generate API/Wire transport, docs and sample together and implement lifecycle, seeded-order, paging, native/wasm and failure fixtures. Coordinate commands/mod.rs with arismoko/horizon#2253/#1863; this note changes no Rust.

## Remaining evidence

Implementation must still verify NeoForge counterpart capabilities (none established), exact vanilla shuffle RNG/draw trace and the seeded permutation oracle, complete integrated Services handoff, transport byte-budget feasibility, transitive predicate immutability and executable panic/trap cleanup. These are evidence and implementation requirements, not unresolved policy choices.
