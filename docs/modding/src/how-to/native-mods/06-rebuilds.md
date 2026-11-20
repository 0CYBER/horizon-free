# Rebuild native mods

A native build uses the instance's pinned Horizon commit. Editing or disabling a mod rebuilds
the mods against that same prebuilt SDK. A mod made for another commit is built when it fits this
one ([What the launcher does](04-what-the-launcher-does.md#mods-made-for-another-build),
[the rule](../../reference/sdk/04-fit.md)); building never repins a mod.

## Update the base

`native-build --update` selects a newer installed release or Nightly and checks every mod against
it. A build without a published SDK says it has no mod tools yet; use a build that has them.

## When a pinned SDK no longer builds

A source pin is not a promise that a later Nightly keeps building the mod. A mod made for an older SDK can pass
the fit check yet fail to compile when the base moves on. The build log is the place to read four things before
changing anything:

- **the mod id** — the failing mod is named in the build step that compiles it;
- **its pinned short hash** — the `[native] horizon` commit the mod declares, repeated in the step that stages
  that source revision;
- **the commit the build actually uses** — the base commit printed at the start of the build, which is the newest
  commit the enabled mods agree on, not necessarily the mod's own pin;
- **the SDK/API version** — `uses.json`'s `schema` and `made_for` (the pin the hashes were taken at), plus the
  hook versions the manifest names. A build with no mod tools yet has no SDK to compile against.

With those four values, repin in the mod's own source rather than in the launcher: set `[native] horizon` in
`horizon-mod.toml` to the commit of the Horizon build the mod should be made for, then run
`horizon-launcher native-build FOLDER --instance NAME --trust-native-code` again. Use `native-build --update` to
move the instance's base to a newer installed release or Nightly first. Repinning is a source change: it changes
what the package says it was made for, so it needs a rebuild and a new release for players.

Older mod formats used `horizon_revision` in a hand-written `horizon-mixin.toml` and `horizon` in
`horizon-native.json`, with `native-build --ignore-pins` for a local rebuild on a different commit. Those files and
that flag are retired: a flat project declares its pin only in `[native] horizon` of `horizon-mod.toml`
([the package](02-the-package.md), [source revisions](../../../../mod-api/native-mod-source-revisions.md)). Convert
an old mod to that layout before repinning; keeping a retired manifest alongside the new one does not choose the
commit.

## Toolchain and storage

The launcher uses its pinned Rust toolchain, not the player's Rust installation. It isolates
compiler flags, Cargo configuration and target settings from the player's environment. Build
inputs and outputs stay in managed launcher storage. On Windows the build tools require paths
the linker can open; the launcher chooses a managed ASCII path when necessary.
