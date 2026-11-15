# Checking a mod against a build's surface

In a mod folder, `cargo horizon-mod` does this for you from the SDK it fetched into `.horizon/sdk/`:

- `cargo horizon-mod check` runs `cargo check` for this computer and for wasm32, checks the package rules,
  and prints what the mod uses: `Greeter uses 14 items of the API (6 hooks, 5 methods, 3 types). Made for
  nightly-2026-11-13 (afe471c).` With `--uses` it lists each item with the line that brought it in.
- `cargo horizon-mod build` packs the list into the package as `uses.json`, with each item's fingerprint at the
  pinned build. A mod with a `[native.*]` table or internals is made for exactly one build and gets no list.
- `cargo horizon-mod fits` says, one line per build, whether the mod fits the current Nightly, or each build
  `--build` names. `cargo horizon-mod fits greeter.hmod --build nightly-2026-11-20` asks the same of a package.
  It exits 0 when every build fits, 1 when one does not, and 2 when it cannot tell or the question does not
  apply (a mod made for one build, a content mod, or a package made by an older tool).
- `cargo horizon-mod update` prints what moving the pin breaks in this mod, item by item with the line that uses
  it and the closest item in the new build, then moves the pin.
- `cargo horizon-mod hooks <text>` searches the pinned build's hooks by id, doc and their NeoForge, Fabric and
  Paper names, offline; `--all` searches every item. `cargo horizon-mod sdk diff <build> <build>` prints what
  changed between two builds, from their small files alone, so it works for builds whose game files are pruned.

Every command and its exit codes are in [Commands](../../reference/sdk/05-commands.md), and the rule the check
applies in [Fitting another build](../../reference/sdk/04-fit.md). The rest of this page is how the same check runs
by hand.

Every build's SDK carries its surface export (`surface.json`). Given the surface
of the build a mod was made for and the mod's folder, write the mod's uses table, then check it against
another build's surface:

```sh
horizon-stage uses MADE_FOR_SURFACE MOD_DIR --json uses.json --made-for BUILD
horizon-stage fits uses.json TARGET_SURFACE
```

`uses` prints each API item the mod uses with the place that brought it in, then how the mod builds each struct
it builds with a literal. The table it writes records, per item, its hash at the made-for build. It also records
the type's trait impls and the public paths the mod writes it by. For each struct the mod names, it records
whether the mod builds it in full, with `..`, or not at all, and the field set when built in full. For a trait's
member, it records every host trait in the mod's list that leads to the member's trait.

`fits` exits 0 when the mod fits (`fits (N items, made for BUILD)`). It exits 1 when it does not, printing
`does not fit: K of the N items it uses changed` and one line per changed item, for example
``the method `PlayersHost::player` (removed)`` or
``the type `DimensionSettings` gained the field `ultrawarm`, and `src/world.rs:12` builds it field by field``.
It exits 2 when an input cannot be read or judged.

The check reads the mod's source with a Rust parser; it is not a compiler and does not expand macros. A name
it cannot resolve with certainty counts as used and as built field by field. A struct literal written with a
path (`world::DimensionSettings { … }`) counts for the API's struct of that name; an unqualified one counts as
the mod's own only in the file that defines it outside any `#[cfg]`, when nothing in that file (a `use`, a glob,
an inline module, a macro or attribute that is not standard or the API's) could change what the name means and
the mod has no `include!`. What the parser cannot see counts as doubt, which refuses rather than guesses:
- inside a macro body, an API struct name followed by `{` or `(` is a build, in full unless it ends in `..`,
  wherever it stands (`a < Pos(1, 2)`, `for Pos(x, y) in v`). A tuple or unit struct's name with no bracket
  after it is a build too, except where only a type stands or as a type written after `name:` in parameters,
  a `let` or a struct's fields (`.map(Pos)`, `{ Marker }` and `make: Pos` count). A literal the scan cannot
  place there counts every buildable API struct the mod names as built in full;
- a `Self` inside a macro body used as a value or followed by `{` or `(`, a `Self { … }` the scan cannot tie
  to an impl, and a literal of a `type` alias whose target is not a plain path, count the same way;
- a file that uses a macro, derive or attribute from outside the mod and the API counts every buildable API
  struct it names as built in full, because the expansion may build it.

So a mod that builds API structs only through a dependency's macro is refused when such a struct gains a field,
even if the macro would cope. One case stays out of the scan's reach: names assembled inside a macro from
outside the mod (`paste!`, `concat_idents!`), which never appear in the source as written. The real compile on
the player's machine is always the final check, and it catches those. Some sources cannot be judged at all:
- a file that does not parse as Rust (the message gives its line);
- source nested deeper than 256 levels. The count is cautious, so one bracket holding more than about 250
  comparisons between plain names (`[a < b, c < d, …]`) is also refused, though it would compile;
- a `use` statement nested deeper than 64 groups;
- a symbolic link;
- a `#[path]` or `include!` that leaves the mod's folder or is computed, or an `include!` renamed by a `use`;
- any source when the scan runs where it cannot give the parser a stack of its own (on wasm).

The parser runs first in a separate process, the same program started again with the mod's source on its
input, never on its command line. If that process runs out of stack, or takes longer than 15 seconds, the
source cannot be judged, and the program that asked carries on. On Linux the process's memory is capped at
4 GiB, and on Linux and macOS its processor time too; Windows and macOS have no memory cap, so there only the
15 seconds bound it.

A mod with a `[native.*]` table or internals is made for exactly one build, and this check does not apply to
it. A mod that fits is still compiled against the target build, and the binary built is exact to that build.
