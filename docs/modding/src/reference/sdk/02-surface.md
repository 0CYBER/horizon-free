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
  has.
- **Conditional**: it breaks only some mods, which the [fit test](04-fit.md) finds from each mod's source. A field
  added to a struct mods can build, or an argument added to a hook (only a mod that writes it out in full); a struct
  no longer constructible; a public path that goes (only a mod that writes that path); a hook's `version` (only a mod
  whose manifest names it); a type's wire form or layout (wasm mods only).
- **Additive**: a new record, and changes that only widen an item: now on wasm, an impl gained, a new public path, an
  item no longer required, an enum no longer `#[non_exhaustive]`.

Each change says what changed in words a mod author can act on, for example
``argument `player`: `PlayerRef` → `ServerPlayerRef` ``. `cargo horizon-mod sdk diff <build> <build>` prints the
same report for two published builds.

When the `verify` check runs on a pull request that changes Rust, its summary carries this report, comparing the
merge base with the head. It reports, and fails the check only when:

- the head's tree cannot be exported;
- a change note cannot be read, or names an item that did not change;
- a breaking change has no change note: every breaking record the report lists, members included (a removed field is
  its own line), is named by a note the pull request adds. Conditional and additive changes need none. The gate is
  `gate = true` in `tools/ci/surface.toml`.

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

## Hook versions

A hook's `version` changes when a listener can see a different meaning under the same signature: its answer now
decides the action, or the order or timing it observes changed. It stays when the engine is only corrected to the
contract the hook already documents. Every bump comes with a change note in the same change, so `sdk diff` and
`cargo horizon-mod update` say what a listener must now do; the version is outside the hook's hash, so the diff
counts a bump as a conditional change for the mods that name that version.
