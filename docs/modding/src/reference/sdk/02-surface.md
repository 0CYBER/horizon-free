# `surface.json`

`surface.json` is the stable surface of one build: everything a mod on the mod API alone can name. It is generated
from the declarations the build parses (`horizon-stage surface ROOT OUT`), never kept by hand or committed, and is
byte-identical for one commit: sorted, with no paths or timestamps.

## Records

Every record has an id, unique in the document, and a `hash` over its contract. Members (a trait's methods, a type's
fields and variants, a hook's arguments) are records of their own:

{{#sdk-surface}}

An item's path is one a mod writes after `mod_api::`: the path of the module that defines it when that module is
public, so a re-export added later never changes its id; otherwise its shortest public path. A record's `paths`
lists every public path a mod can name it by. Types inside signatures are normalised Rust, with paths relative to
`mod_api` and lifetimes kept.

`cargo horizon-mod hooks <text>` searches these records offline ([Commands](05-commands.md)).

## What a hash covers

A record's hash covers its contract, not its `doc`, its `paths`, a hook's `engine` (where it fires) or
`counterparts` (its names in NeoForge, Fabric and Paper), nor the wasm crossing details of a type. Each member has
its own hash, so most additions change no existing hash: a new `Host` method, a new hook, a new hook argument or a
new field on a type change nothing for a mod that doesn't use them. A container's hash covers its members only where
a new member breaks users:

- an exhaustive enum's variants;
- the required methods of a trait mods may implement, and its supertrait bounds;
- a hook's side, kind, scope, whether it is cancellable, its context and host, its owner argument, and whether it has
  an output. Its arguments are member records; its `version` is outside the hash.

A struct's field set is outside its hash. Its `fields_hash` says whether a literal listing every field still builds,
which matters only to a mod that builds it that way. A type's trait impls are a set beside the hash: one gained
changes nothing, one lost is named by the fit test. The document's own `hash` is over every record's id and hash;
`sdk.json` repeats it as `surface.hash`.

`schema` is a number: a reader refuses one it doesn't know and ignores fields it doesn't know.

## What a change is

`horizon-stage surface-diff OLD NEW` compares two documents by id. A record only in the new one is added, only in the
old one removed; in both with another hash, changed. A rename shows as a removal and an addition. Each change is in
one of three classes:

- **Breaking**: it can stop any mod that uses the item from compiling or meaning the same. A record removed; a
  contract changed (a hook's side or kind, a retyped field or argument, a signature, a method no longer on wasm); a
  variant added to an exhaustive enum; a required item added to a trait mods implement; an impl a type no longer
  has; a behaviour removed. Anything that makes the game refuse a wasm mod built before it: an existing call gone or
  with another signature hash (its arguments or answer, or a field of a borrowing record they reach, reordered too), a
  call reaching other owned types, a wire layout change of an owned type an existing call or the core reaches, hidden
  ones too (an enum variant inserted or appended too), and a change to the wire core (`core:wire`: a transport
  value or control payload, a hand-written codec, the NBT tag ids or limits, or a prepared hook's ABI).
- **Conditional**: it breaks only some mods, which the [fit test](04-fit.md) finds from each mod's source (except a
  behaviour's `version`, which no fit report shows: a mod relies on a behaviour without naming it). A field
  added to a struct mods can build, or an argument added to a hook (only a mod that writes it out in full); a struct
  no longer constructible; a public path that goes (only a mod that writes that path); a hook's `version` (only a mod
  whose manifest names it) or a behaviour's `version` (only a mod that relies on it); a type's wire form or layout
  when no existing call reaches it (wasm mods only).
- **Additive**: a new record (a behaviour too), and changes that only widen an item: now on wasm, an impl gained, a new public path, an
  item no longer required, an enum no longer `#[non_exhaustive]`.

Each change says what changed in words a mod author can act on, for example
``argument `player`: `PlayerRef` → `ServerPlayerRef` ``. `cargo horizon-mod sdk diff <build> <build>` prints the
same report for two published builds.

When the `verify (mod points)` check runs on a pull request, its summary carries this report, comparing the merge
base with the head; `--calls` adds the calls and core each build's wasm mods are checked by (`horizon-stage surface
ROOT OUT --calls CALLS` writes them). `--layouts` adds what both builds compiled: the core layout and every call's
hash, exactly as the game compares them (`crates/horizon-mod-api/examples/wire_layouts.rs` prints them). Every
compiled change is reported under the call's id or `core:wire`, even when the source shows a cause (a type the call
reaches, for example): one hash per call cannot tell whether that cause is all of it, so a note names the call or
`core:wire` too. One note can list all of a change's ids. This also catches what the source shows nothing of, a
crossing type's name (the `Wire` derive hashes it, a shape leaves it out) or a change to the generator itself. The
base's compiled layouts come from a store that only main's pushes fill, one file per commit (a symlink, another schema,
no calls or a call count other than the one written beside them is a miss), or are built by checking the base out in place, with
the base's own copy of the example. A base older than the example is compared by source alone, with a warning. A pull
request that changes the gate's own code gets a warning that its result needs a maintainer's look. A source change the
compiled hashes do not see still counts: a newtype that crosses `via` the type it wraps changes the
shape and not the layout, and needs a note. It reports, and fails the check only when:

- the head's tree cannot be exported;
- with the gate on, no comparison can run: the base cannot be found or fetched, the head's tool cannot read it, or
  the head's compiled layouts, or those of a base that has the example, cannot be built;
- a change note cannot be read, or names an item that did not change;
- a breaking change has no change note: every breaking record the report lists, members included (a removed field is
  its own line), is named by a note the pull request adds. Conditional and additive changes need none. The gate is
  `gate = true` in `tools/ci/surface.toml`.

## Wasm mods on a later build

A wasm mod keeps loading on a later build that only added to the API: a method anywhere in a trait, a hook, a
type. Each call the mod makes is its own import, linked by name and signature, and each export is known by its name.
Before the mod runs, the game compares each of the mod's calls with its own, including the Wire layout of every type
the call reaches (an enum's case order too). A hook added after the mod was built is never subscribed; a direct
call of it answers that the mod has none.

The game still refuses a mod, naming the call or point, when:

- a call it makes changed (its signature, or the layout of a type it reaches) or is gone. The generated guest hands
  out `dyn` host traits, and a vtable keeps every method of its trait, so a mod reaches every method of a trait it
  reaches. In practice a changed or removed host method refuses every wasm mod built before the change;
- one of its exports (a hook or a handler method) changed or is gone;
- what every call shares changed: a transport value (`mod_api::wire::PROTOCOL`: reserved IDs, the group state,
  reply tags and limits, and `CORE_VERSION`, bumped by hand for a transport change no value shows), the file
  roots, the prepared ABI, or a hand-written codec, including the NBT tag ids and limits and the `serde` shapes
  some codecs embed (one added counts too). Encodings written outside those files, such as the NBT reader and
  writer, are pinned byte for byte by `horizon-mod-vocab/tests/wire_golden.rs` instead;
- the members of one linked group were built against different builds;
- a point it listens to changed or is gone.

Wasm mods built before calls were linked by name must be rebuilt once.

## Command completion layouts

`CommandSuggestion` now carries an optional `replace_start` byte offset into the prefix. `None` replaces the whole
prefix; `Some(offset)` preserves the text before that offset, which must be on a UTF-8 character boundary and not
past the prefix. This changes the type's Wire layout: wasm mods using it must rebuild against this SDK. The
optional tooltip is retained. See [Server commands](../../how-to/entities/server-commands.md) for replacement
examples and the `id` argument's `tab_complete` completion route.

`CommandArgumentType::suggest` also requires the optional sender before the prefix. Providers own their sender
policy; their returned suggestions remain unfiltered. Implement the required method explicitly, returning an
empty list when the argument has no completions.

## Change notes

A change can carry a note: one file per change in `crates/horizon-mod-vocab/changes/<name>.toml`.

```toml
ids = ["hook:player_logged_in", "type:PlayerLoggedIn.player"]
why = "The hook now fires on the server only."
migrate = "Take `ServerPlayerRef` instead of `PlayerRef`."   # optional
```

A note with an unknown key, a missing `ids` or `why`, an empty `ids`, or an id that did not change is refused. The SDK's `sdk.json`
carries the notes of the changes since the SDK before it (`surface.notes`), and `cargo horizon-mod update` prints a
changed item's `migrate` line beside it.

## Behaviours

A few promises are neither a type nor a signature, so no other record can pin them: how `horizon:add_to` files merge,
or what block entity a block with a vanilla template gets. Each is a `behaviour:<name>` record, generated from one
file in `crates/horizon-mod-vocab/behaviours/<name>.toml` (the file name is the name; there is no list to keep):

```toml
version = 1
doc = "What the engine promises, stated so that a test could check it."
tests = ["crates/horizon-client/src/some_tests.rs::the_test_that_pins_it"]
```

`version` starts at 1 and goes up when a mod that relies on the behaviour can see a different meaning. `tests` lists
the tests that pin the promise, each as `<file path from the repository root>::<fn name>`; the export refuses a behaviour whose
test has no `fn` of that name, and a malformed file, naming it. `doc` and `tests` are outside the record's hash.

The diff treats a behaviour like any record: a new one is additive, a removed one is breaking (so it needs a change
note that names `behaviour:<name>`), and a `version` change is conditional, for the mods that rely on it. Nothing in a
mod's code uses a behaviour, so it never appears in a fit report. Only promises kept across builds belong here, not
diagnostics or behaviour that vanilla fixes.

## Hook versions

A hook's `version` changes when a listener can see a different meaning under the same signature: its answer now
decides the action, or the order or timing it observes changed. It stays when the engine is only corrected to the
contract the hook already documents. Every bump comes with a change note in the same change, so `sdk diff` and
`cargo horizon-mod update` say what a listener must now do; the version is outside the hook's hash, so the diff
counts a bump as a conditional change for the mods that name that version.
