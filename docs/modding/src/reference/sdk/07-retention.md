# Retention

An SDK release is never deleted whole. Pruning deletes only the assets whose `kind` in the release's signed
`sdk.json` is `runtime`, `api` or `engine`. A pruned build keeps `sdk.json`, `surface.json`, `build-data.tar.xz`,
`api-source.tar.xz`, `SHA256SUMS` and `SHA256SUMS.sig`, so for it:

- `cargo horizon-mod sdk diff` and `fits`, and the launcher's fit test, keep working;
- a mod project pinned to it still fetches its `.horizon/sdk/`;
- building a mod against it on a player's computer needs its compiled slices, which are gone.

The set kept whole is every build a current or dated Nightly ships, and the ten newest other SDKs. Automatic pruning
is not switched on yet: until it is, every published SDK stays whole. The policy, with how the kept set is computed,
is in the release flow's
[retention design](https://github.com/horizon-rust/horizon/blob/main/docs/release-flow.md#prebuilt-sdk-retention-design-horizon-rusthorizon-3011).
Releases of the older `prebuilt-<sha>` form are outside it and removed by hand.

The launcher keeps its own copies under its own rules ([Cache and disk](../../how-to/native-mods/10-cache-and-disk.md)):
a build's small files are kept, so a mod's fit is told offline, and its compiled slices are evicted with the build.
