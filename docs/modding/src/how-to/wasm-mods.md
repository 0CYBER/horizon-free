# WebAssembly mods

A mod written against `mod_api` builds either way from the same source: native, compiled against
the pinned prebuilt game by the launcher ([Native mods](native-mods.md)), or to `mod.wasm`, which runs in a game with
a compatible API layout and wasm toolchain. Only the build step differs: the hooks, Parts,
`[dependencies]` and the manifest mean the same for both.

Use the `server_tick` hook to count ticks:

```rust,ignore
{{#include ../../examples/wasm-hello/src/lib.rs:mod}}
```

Ordinary wasm calls wait up to five seconds for the group, including while the
group is lending its store to a host import. Nested calls refuse immediately if
they would have to wait on a lent store, which can form a lock cycle. Worker
calls wait without this bound. Same-thread reentry shares the enclosing call's
remaining deadline. A contended prepared release invalidates the token
immediately and runs the guest cleanup on the next call into that mod.

## Owned values and `Wire`

Native and wasm use the same owned-value encoding. Choose which of your own
values may cross that boundary by explicitly deriving `mod_api::Wire`:

```rust,ignore
#[derive(mod_api::Wire)]
struct CounterUpdate {
    counter: mod_api::Identifier,
    value: u32,
}
```

Records write fields in declaration order; enum tags are zero-based declaration
indices, not Rust discriminants. Each field must have a Wire form. Borrowed hook
arguments are call-table views, not owned Wire values; a borrowed record cannot
use this derive. The derive is never added automatically to a mod's types.

For a value with checked construction, `#[wire(via = SomeOwnedType)]` uses
`From<&Self>` to encode and `TryFrom<SomeOwnedType>` to decode. A failed conversion
refuses the value rather than constructing unchecked state. This also lets a type
retain a deliberately chosen encoding instead of exposing its Rust fields.

The loader compares composed encoding layouts, including nested fields and
recursive shapes. Equivalent standard representations share one encoding:
`Vec<T>` and `Box<[T]>`, for example, are both lists, while `Box<T>` is transparent.
Changing a field's encoded form changes its callers' layout too. Layouts are
computed once at load; they are not per-tick work.

## Build

```sh
cargo horizon-mod build --wasm
```

The command builds your crate with the pinned Rust toolchain (installed through rustup the first time, with
`rust-src`), writes `mod.wasm` beside `horizon-mod.toml`, records it in a `[wasm]` section, and
packs `target/horizon-mod/<id>.hmod`. Let the tool write `[wasm]`; do not edit it by hand:

```toml
[wasm]
sha512 = "…"                      # mod.wasm's hash, checked when the game loads it
toolchain = "1.99.1 <commit>"     # every mod linked together needs the same one
[wasm.linked]                     # the exact builds of its wasm dependencies
my_library = "…"
```

A mod carries native code or wasm code, not both. A wasm mod has no `[native]` table.

A mod with `[[inject]]` listeners builds against the game's staged point catalog. Pass
`--catalog path/to/points.json`, or set `HORIZON_WASM_STD` to the toolchain's std member
with `points.json` beside it. Mods without listeners need no catalog. When packaging the
wasm toolchain, `cargo horizon-mod wasm-std <out.wasm> --catalog <staged-points.json>`
copies that catalog beside the std member.

A dependency group shares its handler table and cache budget. A handler registered by one
member is usable by other members of that group.

Use `mod_api`'s wasm allocator and standard collections. When an allocation
fails because the group's memory is full (see [Memory](#memory)), it calls every callback registered with
`mod_api::wire::guest::register_eviction(fn())` once and retries once; if that frees nothing the allocation fails as
usual (`try_reserve` returns an error, an infallible allocation ends the call with a trap). Register a callback to
give back what a cache can rebuild. It must free memory and must not allocate. Registering the same function twice is
harmless; `false` means all 29 slots are taken.

A wasm mod must not declare its own `#[global_allocator]`: `cargo horizon-mod build --wasm` fails with rustc's error
"the `#[global_allocator]` in this crate conflicts with global allocator in: mod_api". Allocate through the standard
collections and let the group's allocator do the rest.

## Dependencies

`[dependencies]` work as for native mods. List the library mod in `horizon-mod.toml` and its
crate in `Cargo.toml`. The tool stages the resolved mod packages into one Cargo workspace:
each library's single compiler unit produces both its rlib and its position-independent member,
and dependents use that exact rlib through Cargo's ordinary dependency edges. Cargo owns
feature resolution, build scripts and non-mod dependencies. Registration generation is scoped to
each package, not to the mod from which the command was launched.

Ordinary Rust calls such as `my_library::function()`, statics, generics and owned Rust types need
no generated interface or export annotations. Members export their public definitions without
symbol interposition: duplicate definitions are refused, rather than overriding another mod.
Each member records its compiler, target, settings,
rlib digest and dependency artifact identities in `horizon.build` metadata covered by its module
hash. Mod crate disambiguators use the mod ID, version and resolved Cargo features rather than
the temporary workspace path. A shared crate disambiguator alone is not an ABI guarantee:
the loader checks the exact dependency build recorded in `[wasm.linked]`, including when the
library and its consumers were built separately. Changing a library's source, toolchain,
features or settings requires rebuilding its consumers.

The build packages every member of the resolved group into the invoking project's
`target/horizon-mod/` directory. A consumer build does not replace another project's previously
packaged `.hmod`. To include additional local roots in the same Cargo invocation, repeat
`--with path/to/another/mod`. The game links a mod with its
dependencies at load, one copy of each library shared by every mod that uses it, and refuses a
mod whose library is missing or another build than the one it was linked against, naming it on
the Mods page. An optional dependency is built in with the Cargo feature of its id, so its crate's
entry in `Cargo.toml` is `optional = true` under that id; an absent one is left out of the group at
load, and `if_installed!` keeps the mod from calling it.

## Injection points

API-typed `[[inject]]` listeners use the same generated `mod_points` records and constructors as
native listeners. When injections are declared, the wasm build requires the pinned game's staged
`points.json`, supplied via `--catalog` or beside the configured std member.
Listeners register in the host's ordinary point list, with the same priority, dependency order,
owner enable flag and stop behavior as native listeners. Mixins are native-only.

A point's record is a copy of every parameter as an API value, `&mut` parameters copied back
after the listener, exactly as for a native listener; a point with a parameter that has no API type
refuses the listener at registration, naming the parameter. The `access` key on `[[inject]]` is refused by name on both targets. Native code needing game internals uses `[[native.mixin]]`; wasm cannot declare `[[internals]]` or native transformations.

## ABI reference

A dependency group consists of core wasm shared modules with `dylink.0`, not components.
The loader provides one linear memory, function table and standard-library member per group,
plus the `env` base globals and the `GOT.mem` and `GOT.func` relocations used by the members.
The loader supplies linker globals such as `GOT.mem.__heap_base` and `GOT.mem.__data_end`;
member constructors run after linking. Groups do not share memory with one another.

The build step moves a core start function to the explicit `__wasm_start` export. The loader
instantiates members, fills self-referencing GOT entries from their actual exports, then runs
`__wasm_start`, data relocations and constructors in that order. A raw core start is refused:
it would execute before its own addresses could be relocated. `horizon.private` metadata lists
compiler implementation exports that remain local to a member (for example an upstream inline
method emitted in several members). Their original symbols can resolve that member's own GOT,
but are not published as library definitions. This is symbol visibility metadata, not function
glue or another Rust interface.

Pointers and lengths below are wasm32 `u32` values. The game provides these imports in the
`horizon` module:

- `host(id, in_ptr, in_len, out_ptr, out_cap) -> u32` invokes a generated import ID. Arguments
  are Wire values in order. It writes the reply if it fits and returns its length; if the length
  exceeds `out_cap`, call `host_reply(out_ptr)` after allocating that many bytes to copy the
  retained reply. Host errors trap the call.
- `host_reply(out_ptr)` copies that retained reply.

Each guest exports:

- `horizon_layout() -> u64`: the generated layout hash. A mismatch is refused.
- `horizon_layout_ext() -> u64`, when `mod_points` is linked: the full point catalog's layout
  hash. The loader checks it before accepting any listener.
- `horizon_in(len) -> ptr`: the buffer into which the game writes the next call's input.
- `horizon_hook(id) -> u64`: runs the generated export ID after the input is written. The high
  32 bits are the reply pointer and the low 32 bits its length; `u32::MAX` as the length denotes
  an error, with a Wire string at the pointer. The reply remains valid until the next call at
  the same depth. A handler call starts its input with the handler's `u64` ID.

The reserved IDs are `INIT = 0xFFFF_FF00` (guest package-list initialization),
`RELEASE = 0xFFFF_FF01` (guest handler IDs to release), and `HOST_RELEASE = 0xFFFF_FF02`
(host object ID to release), and `HOST_LOG = 0xFFFF_FF03` (a guest panic string, with no reply).
Other IDs index the generated import or export table. A hook reply
contains its return value followed by writable arguments in field order.

Points use that same hook export, call arena and execution budget. Place IDs are
`0x8000_0000 + place`. The subscription query `0xBFFF_FFFF` returns `(place, injection_id)`
pairs. Point call input starts with the `u32` subscription index. A `HostSide` point then carries
a server/client boolean, followed by its listener record. Calls reply with unit and the record,
which the host decodes completely before writing any field back.

Wire values use little-endian numbers; lengths and collection counts are `u32`, strings are a
byte length followed by UTF-8 bytes, options have a byte tag (`0` or `1`), enums have a `u32`
case followed by its fields, and records contain their fields in declaration order. Readers reject
invalid tags, invalid UTF-8, trailing data, lengths that cannot fit in the remaining bytes, and
structural nesting beyond `mod_api::wire::MAX_DEPTH` (32 levels). `Tag` and `Text` use
separate, stack-independent tree codecs that preserve vanilla's depth limit of 512. Interned static strings are limited to 256 bytes and 4096 distinct
strings per process. Execution, memory and table limits are enforced separately by the loader.

## In the game

Each dependency group uses shared memory and one copy of the standard library; its members are
core wasm shared modules, not components. A layout mismatch or an unresolved dependency is refused
and named on the Mods page.

The game limits execution time, memory and table usage. A trap or limit hit disables the affected
group. APIs marked `Not on wasm` in their Rust documentation cannot be used by wasm mods.

### Memory

A group's members share one linear memory: their heaps, stacks and static data together. Its limit is
the largest `wasm_memory_mib` a member's `horizon-mod.toml` declares (1 to 4096 MiB, wasm32's 4 GiB),
so a library declares what it and its addons need. Without a declaration, the limit is
1 GiB or the instance cap, whichever is smaller. The limit is a ceiling, not memory taken: the group
uses what it grows to. It bounds that used memory only: on 64-bit systems the runtime still reserves
address space for each memory (4 GiB plus a guard region), committing none of it, and a lower cap does
not shrink that reservation. Past the limit, or past what the system gives, `memory.grow` answers -1 inside the
guest and the call goes on; Rust's allocator then reports the failure as usual (`try_reserve` returns an
error; an infallible allocation ends the call with a trap and the log names the group and its limit).

Each instance caps every group with `wasm.memory_cap_mib:<MiB>` in `horizon-mods.txt`, beside `options.txt`:
4096 unless the player lowers it. A group whose members declare more than the cap is refused at load, and
the Mods page names the group, what it declares and the cap.

### Threads

Wasm threads and shared memory are off for wasm mods. A module that declares or imports a shared memory, or uses an atomic instruction, is refused when the group links, and the refusal names the member and quotes the reason. Wasmtime's `threads` feature is compiled in only for native hosts that embed their own engine; the engine that runs mods keeps both settings off.

## Compiled module cache

The game caches compiled wasm modules in `<game_dir>/.horizon-cache/wasm/`, keyed by the
wasm bytes, Wasmtime compatibility and engine settings. Hits map the compiled file into memory;
files shipped beside a mod are never loaded as compiled code.

On Unix, a cache miss compiles in a short-lived child of the game executable, returning
compiler memory to the operating system when the child exits. The internal entry runs before
game initialization, with closed standard streams and an empty environment. It receives a
private input file and the expected cache key, refuses changed input, and writes through the
same atomic cache writer. The parent accepts only its expected entry through the verified
opened-file loader. Startup failure, a crash, a non-zero exit or an invalid entry
refuses the module with an actionable compiler/cache diagnostic: the game never
retries without the applied budget. A timeout names the exceeded limit (not a
crash), kills and reaps the child, and removes its private input and unfinished
writer files. It never records a permanent failure: restarting with a larger
limit can compile the same module. This child path is Unix-only; Windows continues
to compile in process with persistent caching disabled. An unavailable or untrusted
Unix cache refuses compilation until its private writable directory is restored.

### Compilation time setting

Set `wasm.compile_timeout_secs:120` in the client's `options.txt` (under the resolved
`--player-dir` when supplied, otherwise the game directory), or
`wasm.compile_timeout_secs=120` in the dedicated game directory's `server.properties`.
With the server supervisor, put `"wasm.compile_timeout_secs" = "120"` in
`server.toml`'s existing `[properties]` table instead. Restart to apply the setting.
The value is positive finite seconds (fractions allowed), representable as a duration.
Zero, non-numeric and out-of-range values refuse wasm loading, naming the setting
and the exact value in the log, console and Mods-page refusal registry; the refused
package's content is excluded too. They never silently select a default. Remove
the line to restore automatic sizing. The client preserves this startup setting
when saving its options; no launcher rebuild or game argument is needed.

The unset default is `5 + 300 / available_threads` seconds (at least one thread):
**1 → 305 s; 2 → 155 s; 8 → 42.5 s; 64 → 9.6875 s**.
Issue #3673 measured a 30 MB module at roughly 30 s on one thread and 3 s on eight.
The 300 s single-thread allowance is ten times the former, plus 5 s fixed headroom.
The eight-thread default remains above the previous 37.743749290 s limit, which
was ten times a 3.774374929 s compile of a 67,998,240-byte module on a Ryzen 7 5700X.
These are conservative budgets, not promised compile times. A timeout names the
package/module file, applied limit, whether it was configured or the default for
N threads, and the setting to increase.

On Unix, the game creates the cache folder with mode 0700 and entries with mode 0600. It checks
folder ownership and permissions, then checks each opened entry is a regular file owned by the
current user, not writable by group or others, and has only one hard link. Symlink entries are
refused. On Windows, persistent caching is disabled until owner, DACL and inheritance trust
can be verified for the cache folder and opened entries. Modules compile from wasm instead,
with one warning explaining the fallback. The player's account is the trust boundary.

At startup, the game removes regular `.cwasm` files from other engine configurations or
Wasmtime versions, along with malformed and old-format cache names. It never follows symlinks
for cleanup. Current-engine entries have a 1 GiB startup budget: when their combined size
exceeds it, entries are removed oldest first by modification time until within the budget.
Current entries are not removed just because they are old. Entries written during this launch
are retained until a later startup. The proposed budget allows 30 copies of the largest
measured compiled module (34,039,320 bytes) plus headroom; the 30-module allowance awaits
maintainer confirmation and does not represent a measured pack.

A failed entry check logs the reason once per launch. Missing, corrupt or incompatible
entries trigger a budgeted compiler child on Unix, provided the cache directory is
trusted and writable. Writes replace entries atomically without changing files that
running modules have mapped. Deleting the cache is always safe: the next launch rebuilds it.

## Guest stack

Every wasm mod has the same 128 MiB guest stack limit; manifests cannot change it.
Stack overflow traps and names the mod. Guest-entering threads also reserve host stack space.
If the required stack cannot be reserved, startup is refused with `GuestStackUnavailable`.
