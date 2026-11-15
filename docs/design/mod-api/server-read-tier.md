# Design: server read tier for handles

Proposed contract for [arismoko/horizon#2809](https://github.com/arismoko/horizon/issues/2809).
This is a design, not an implemented guarantee. Recommend option A: immutable
server reads take `&dyn ServerHost`; shared reads retain `&dyn Host`.

## Current ownership and the gap

Evidence is from main at `d5b9b22cd1`. The declarations now live in
`crates/horizon-mod-vocab/src/host`, despite older generator comments naming
horizon-mod-api. `crates/horizon-mod-api-gen/src/host.rs:159` owns that directory.
The same file's `Tier` at line 165 has Backend, ClientBackend and their mutable
variants, but no ServerBackend. `area_tiers` at line 247 classifies by trait
name; Entry at line 221 retains attributes but no parsed side.

`handle_method` at line 749 matches the first argument's handle type and the
method prefix. Its receiver selection at line 781 uses mutability and
`tier.client()` only: every non-client immutable read takes `&dyn Host`.
Its forwarding body at line 808 uses `backend_of` for reads. The generation loop
at line 878 emits handle methods and gathers Backend/ClientBackend reads.
`HostSide` at line 903 carries mutable server/client host borrows; the generated
unions at line 923 expose the shared backend from Host. A server label in prose
therefore cannot restrict a client caller.

Players illustrate the hole: `crates/horizon-mod-vocab/src/host/players.rs:42`
declares PlayersBackend, and line 48 declares `player_recipe_book_settings` under
`Side: server`. PlayersServerHost at line 22 supplies services, not a distinct
immutable handle backend. Moving a label alone cannot change the generated
receiver. [PR arismoko/horizon#2784](https://github.com/arismoko/horizon/pull/2784) motivates the
level/dimension example, but those additions are not in this main snapshot;
they must join the same inventory if merged before implementation.

The inventory of immutable server-labelled declarations with a handle first
argument is 255 reads (including multiline signatures):

| Handle | Reads | Handle | Reads |
|---|---:|---|---:|
| BlockEntityRef | 2 | BossBarRef | 11 |
| ChunkRef | 4 | DisplayRef | 24 |
| DragonRef | 3 | EntityRef | 53 |
| FishingHookRef | 3 | ItemEntityRef | 5 |
| LevelRef | 48 | LivingRef | 12 |
| MenuRef | 5 | MerchantRef | 6 |
| MinecartRef | 13 | MobRef | 18 |
| PlayerRef | 22 | ProjectileRef | 5 |
| RaidRef | 14 | UnspawnedMobRef | 7 |

Reproduce from the fetched main ref with this single shell command (prints each
handle count and the total; no build required):

```sh
python3 -c 'exec("import subprocess,re,collections\nc=collections.Counter()\nfor p in subprocess.check_output(['"'"'git'"'"','"'"'ls-tree'"'"','"'"'-r'"'"','"'"'--name-only'"'"','"'"'arismoko/main'"'"','"'"'--'"'"','"'"'crates/horizon-mod-vocab/src/host'"'"'],text=True).splitlines():\n s=subprocess.check_output(['"'"'git'"'"','"'"'show'"'"','"'"'arismoko/main:'"'"'+p],text=True)\n for t in re.finditer(r'"'"'pub trait \\w*Backend\\s*\\{(.*?)\\n\\}'"'"',s,re.S):\n  for m in re.finditer(r'"'"'/// Side: (server|both|client)\\.(.*?)(?=/// Side:|\\Z)'"'"',t[1],re.S):\n   f=re.search(r'"'"'fn\\s+(\\w+)\\s*\\(\\s*&self\\s*,\\s*\\w+\\s*:\\s*(\\w+)'"'"',m[2])\n   if f and m[1]=='"'"'server'"'"' and f[2].endswith(('"'"'Ref'"'"','"'"'Key'"'"')) and f[1].startswith(re.sub(r'"'"'(?<!^)(?=[A-Z])'"'"','"'"'_'"'"',re.sub(r'"'"'(Ref|Key)$'"'"','"'"''"'"',f[2])).lower()+'"'"'_'"'"'): c[f[2]]+=1\nprint(dict(sorted(c.items())));print(sum(c.values()))\n")'
```

This gives 255, versus the earlier inventory's 252 across the same 18 handles.
LevelRef 48 and EntityRef 53 agree; DisplayRef is 24 here versus the earlier inventory's 23.
The other two differences cannot be attributed without that inventory in full
and revision. The command pins the counting rule to the fetched ref rather than
assuming those totals agree. The earlier 204 undercount used semicolons as
method boundaries; semicolons inside documentation truncated the side label.
This command uses side-labelled blocks within Backend traits instead.

Recompute with the generator's parsed handle map and prefix rule before rewriting;
these counts are a review baseline, not a hard-coded migration allowlist.

## Alternatives

**A — immutable ServerHost receiver (recommended).** Add ServerBackend to the
tier model, with an immutable accessor on ServerHost and a separate generated
backend union. Move server reads into each area's ServerBackend. Generate
`PlayerRef::address(self, host: &dyn ServerHost) -> Option<String>` and equivalent
signatures for every inventoried read. Reads labelled both stay in Backend;
client reads stay in ClientBackend. No mutable borrow is needed for a read.
The shared backend must no longer contain server methods, including through
hidden accessors: changing only the public forwarding signature leaves a bypass.

Server mod code is `player.address(host)` with `host: &dyn ServerHost`.
Client code with `host: &dyn ClientHost` fails with E0308 at `address`:
`expected trait ServerHost, found trait ClientHost`. The expected diagnostic
includes the source call `player.address(host)` and the definition of `address`.
This is the required read-name evidence; do not promise identical rustc prose
across compiler releases. A direct trait-object mismatch does not use
`#[diagnostic::on_unimplemented]`. Capture actual stderr on the pinned compiler
in implementation, including the method name and expected/found host types.

A both-side hook matches HostSide: in Server it can call server reads and
shared reads through an immutable reborrow; in Client only shared and client
reads compile. Shared reads use Host upcasting in either arm. Do not add a
HostSide conversion that erases the side and then restores server authority.
Read-only server hooks currently receiving only Host need an immutable
ServerHost parameter; audit `crates/horizon-mod-api-gen/src/host_param.rs:107`
and scoped goal views alongside handle signatures so legitimate callers retain
the required capability.

The audit must also include level-scoped hooks and Part callbacks: LevelSide,
LevelHost, LevelTicksHost, LevelRandomHost and LevelWriteHost. None of those
traits extends ServerHost today: see
`crates/horizon-mod-vocab/src/world/types.rs:1147` (LevelHost),
`crates/horizon-mod-vocab/src/world/types.rs:1266` (LevelTicksHost), and
`crates/horizon-mod-vocab/src/world/types.rs:1304` (LevelWriteHost).
`crates/horizon-mod-api-gen/src/host_param.rs:125` selects LevelSide or the
level host family; `crates/horizon-mod-api-gen/src/host.rs:910` currently gives
both LevelSide arms the same `&dyn LevelHost` type.

Proposed rule: server-only reads must be reachable through an explicit immutable
server-read capability on server-side level hosts. Recommend a small shared
server-read supertrait/accessor rather than requiring the entire ServerHost
service surface on a scoped level view. Server-side level views implement that
tier subject to their existing scope refusals; the client LevelHost must not.
Split the LevelSide payload types accordingly. Do not add server authority to
the shared LevelHost merely to make callers compile. This remains a proposal:
the implementation must settle the exact capability trait and its receiver
conversion together with option A, update Part callback signatures and both
native/wasm adapters, and compile-test server level access and client rejection.


**B — typed side or sealed marker.** Replace the host vocabulary with
`Host<Server>` and `Host<Client>`, with a sealed Side and server-only marker.
A shared read is generic over either side; `address` requires
`S: CanReadPlayerAddress` implemented only for Server. A proposed
`#[diagnostic::on_unimplemented(message = "PlayerRef::address requires a server host") ]`
on that sealed bound produces E0277 with
`PlayerRef::address requires a server host` for Client. Server code remains
`player.address(host)` and compiles through the Server implementation.
The generic bound must occur on the handle method, without generic methods on
the object-safe host trait. HostSide becomes an enum of the two concrete side
instantiations; callers still match it. Shared helper functions become generic
or explicitly erase only shared authority. There must be no blanket marker
implementation admitting Client or user-defined Side values.

For both options, native and wasm use identical generated public signatures.
Native adapters implement the new backend split. Wasm guest trait impls,
import ownership and host dispatch must split too: inspect
`crates/horizon-mod-api-gen/src/wire/calls.rs:213` (guest generation) and
`crates/horizon-mod-api-gen/src/wire/mod.rs:72` (host access mappings).
A client execution context must refuse a forged server-read import before
backend dispatch. Static typing is not a sandbox against handcrafted wasm.

A fits the existing ServerHost: Host and ClientHost: Host contract with less
public generic machinery. B offers a custom diagnostic but requires changing
all host-bearing abstractions and adds per-read marker machinery. Prefer A;
method-name stderr coverage satisfies the issue without that expansion.

## One scripted clean break

Commit a reproducible implementation script with the eventual clean break. It must parse declarations, require exactly one valid Side for
every read, inventory handles using the generator's recognition rules, and
move all server reads together. Preserve server/both/client labels; normalize
missing or contradictory labels only after explicit review, never infer a
side from a method name. Rewrite trait membership, generator tier/signature
rules and backend accessors, engine implementations and callers, scoped views,
fixtures, horizon-machine-sample and documentation in the same change.
Update CLAUDE.md's reads bullet to describe all three receivers.

The script must fail on unrecognized declarations and unresolved callers,
provide a check mode, and be idempotent. Regenerate with
`cargo run -p xtask -- mod-api` through the prescribed build lane during
implementation. Verify a second run has no diff and compare the before/after
inventory by full read identity. No aliases, compatibility forwarding methods,
or per-read incremental rollout. Refresh wasm coverage and interface hashes
through their existing generators; unsupported wasm capabilities retain their
named registration refusal, never a client fallback.

## Tests and acceptance

Use repository Rust tests beside generator code and its existing fixture crate;
compile fixtures with the pinned compiler, not a Python script extracting Rust.
For every populated backend area/kind, select a server read for a client
compile-fail case and a paired server compile-pass case. Cover Backend being
split into ServerBackend, retained shared Backend, and ClientBackend; synthetic
fixtures cover an empty tier combination. Mutable backend signatures must stay
separately covered. Run both native and wasm target checks in implementation.
A documentation test must enumerate every immutable read, require exactly one
Side label, and compare generated rustdoc's label and receiver with that side.
Do not satisfy it with a fixed list of 255 methods.

| Example | Expected result |
|---|---|
| `player.address(client_host)` | E0308, names `address`, expects ServerHost |
| `player.address(server_host)` | Compiles; preserves existing Option result |
| `player.recipe_book_settings(client_host, book)` | E0308 names `recipe_book_settings` |
| Same read with server host | Compiles |
| Shared read in either HostSide arm | Compiles with shared Host reborrow |
| Server read in HostSide::Client arm | Compile failure |
| Server read with erased `&dyn Host` | Compile failure; no downcast escape |
| Server read on server host with absent backend | Existing typed refusal |
| Forged wasm server import in client context | Refused before backend call |
| Repeat migration and regeneration | Zero diff |

This adds no hook, listener, identity, savepoint, journal or registration payload.
Existing handles keep their lifecycle holder and generation. Mod inputs remain
untrusted: preserve existing bounds and API-owned Wire values, decoding budgets
and stale-handle refusals. Do not widen vanilla capacities. Existing panic and
transaction policies remain in force; wrong-side calls must never be implemented
as a panic or runtime fallback. The tier adds no allocation, listener scan or
per-tick branch; unused cost is O(1), with zero additional runtime work for
native static receiver checking.

Vanilla 26.3 has no Rust Host receiver contract; this proposal changes API
accessibility, not gameplay or a vanilla call site. The issue identifies no
NeoForge counterpart. No NeoForge 26.3 public contract or limit is asserted here;
parity for such a contract remains unverified and would require public docs or
javadoc before adoption. No other project's source is needed for this change.

## Stages and open questions

1. Review this contract and baseline inventory (files: none).
2. Land the committed rewrite script, generator split, all caller updates,
   native/wasm boundary changes and tests atomically (no transitional release).
3. Run regeneration, compiler fixtures, documentation checks and affected crate
   checks, then review inventory and generated API diff before publishing.

Open question: should errors require a custom sentence beyond the read name?
Recommend accepting A's E0308 with the named source call and definition;
choose B only if a custom diagnostic is an explicit product requirement.
Actual compiler stderr and forged-import refusal ownership must be verified in
the implementation; they are acceptance work, not evidence of current behavior.
