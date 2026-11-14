# Conflict hotspots

A file that many independent PRs keep editing (a hand-kept list, a registry, a giant module) becomes a generated
list or one file per item, so independent PRs stop conflicting. These scripts keep new long lists
out, and carry old-style PRs across a switch. Standard library Python and git only.

| Script | For |
|---|---|
| `carry.py` | Writes an old-style PR's addition to a switched list in the new form, into the working tree. |
| `check_lists.py` | CI's `verify`: a `pub use` brace list under `crates/horizon-mod-vocab/src/exports/` names at most 12 items; the longer ones in its table may not grow past their entry (shortening needs no edit). A fragment a switch removed (`part_table.rs`) may not come back. |
| `check_stand_ins.py` | CI's `verify`: the dispatch tests' stand-ins are one file each (`fixtures/stubs/<Name>.rs`, `fixtures/api/<Name>.rs`, a name in one folder only, none back in `shared.rs`); every type a hook signature names has one; the wire loopback's hand-written impls of vocabulary traits have every required method, and of `CapabilityProviders` every member trait. Its docstring lists what it cannot see (names the generator adds, derives, stub methods). |

## In a rollup, after a switch

A member PR written against a switched list conflicts on the old file.

The vocabulary's type lists in `crates/horizon-mod-vocab/src/exports/domains.rs` are one glob per domain
(`pub use <domain>::types::*;`): every `pub` item of a `<domain>/types.rs` is API by its keyword. A member PR that
added a name to one of the removed lists conflicts in that list: take main's side of the list after checking that the
PR's diff defines the name as a `pub` item in that `types.rs`. No carry script is needed. Any other hunk of the PR in
`domains.rs` (a `mod` line, a `backend` or `runtime` re-export) is outside the lists and merges as before.

For the other switched lists, take main's side of the old file, then:

```sh
git fetch origin pull/<N>/head:refs/pull/<N>/head
python3 tools/hotspots/carry.py ignored-test <N>             # or --topic <name>; --head pr/<N> for another ref
python3 tools/hotspots/carry.py dispatch-stub <N>            # a stand-in added to the old shared.rs stubs/api modules
python3 tools/hotspots/carry.py part-table <N>               # a name added to the old exports/part_table.rs
git add <the files it names> && git commit -m "Carry for #<N>: <what moved>"
```

It writes nothing and prints the hunk when it cannot place one: carry that hunk by hand. `shared.rs` itself stays
for `dispatch-stub`: take main's side only of its hunks inside the old `mod stubs` and `mod api`. `part-table`
writes each name the PR added to the old `crates/horizon-mod-vocab/src/exports/part_table.rs` into
`exports/parts_<file>.rs`, the fragment of the parts file that defines it on the PR's head, in the same scope and
`#[cfg]`; run `rustfmt --edition 2024` on the files it names. Taking main's side alone would drop the export, so a
forgotten carry fails to compile.
