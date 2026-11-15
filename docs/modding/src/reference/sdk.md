# The mod SDK

The mod SDK is everything needed to write, check, build and load a code mod for one exact game build. Each build
that has one publishes it as one signed release, tag `sdk-<build id>`, where the **build id** is the game's full
commit sha. People see builds by their Nightly names: `nightly-2026-11-13 (afe471c)`, or the short sha for a build
no Nightly shipped.

- A mod author's project holds the small part of it: the API crates, with their generated code already written out,
  in `.horizon/sdk/`. `cargo horizon-mod` fetches and verifies it there
  ([In a mod project](sdk/03-in-a-project.md)); no clone of the game and no generator run.
- The launcher fetches the compiled part a build needs on the player's computer, after the same checks
  ([What the launcher does](../how-to/native-mods/04-what-the-launcher-does.md)).
- Tools outside Horizon read `sdk.json`, check the signature, then read `surface.json` and the build data. They need
  no source tree.

A mod is **made for** one build, the one it pins. A mod on the mod API alone also **fits** every other build where
nothing it uses changed, and the launcher compiles it there ([Fitting another build](sdk/04-fit.md)).

<!-- section-pages -->
