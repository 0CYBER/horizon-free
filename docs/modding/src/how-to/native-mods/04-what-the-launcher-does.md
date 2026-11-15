# What the launcher does

Native mods run with your rights. Read their source and dependencies before allowing native code.
Install them into the selected instance from the Mods page, then choose **Build**, or run:

```sh
horizon-launcher native-build path/to/mod --instance INSTANCE --trust-native-code
```

Omit the folder to build the instance's enabled native mods. The launcher keeps the instance on
its pinned Horizon commit; `--update` selects a newer installed base and rebuilds the mods against
that commit's SDK.

## Build steps

1. Copy the mod projects into an immutable incoming directory and validate their pins and dependencies.
2. Read the game build's id from its release's signed `sdk.json`, then the SDK's own `sdk.json`
   (`sdk-<build id>`), checked against its signed `SHA256SUMS`. A schema, package format or Rust
   newer than the launcher reads asks for a launcher update.
3. Check each mod made for another build against this one (see "Mods made for another build"
   below). A mod that doesn't fit is left out of the build and named, never removed.
4. Download only the SDK slices the instance's mods need: `runtime` and `api` when every mod uses
   the mod API alone, `engine` as well when any mod has a `[native.*]` table, internals or a
   registry dependency. Each slice is checked against `sdk.json`, unpacked into its own folder and
   used again on later builds after a quick check of its file names and sizes. Install the
   launcher's pinned Rust toolchain and required linker tools if needed; the player's Rust is not used.
5. Compile the mods against the SDK's game libraries. Registry dependencies use the mod's supplied
   lockfile and the same immutable SDK. The launcher asks for permission before executing dependency
   build scripts or procedural macros.
6. For `[[native.mixin]]` rows, download the pinned game source, copy the targeted functions into one patch
   library, apply the rows with horizon-mixins, and compile that library against the SDK. The game
   crates are not compiled. Each checked replacement is installed through its typed installer.
7. Link the native executable, install it as `native-<commit>-<key>`, and select it for the instance.
   `native-mods.json` records the base, pins, mods and build tools. Client builds share their base's
   prepared resources; server exports include their resources.
   On Windows x64 the launcher also places the two GNU runtime libraries of its managed toolchain
   (`libgcc_s_seh-1.dll` and `libwinpthread-1.dll`) beside the executable, so it starts without a
   separate compiler installation.

A matching installed build is reused. A failed build stops with its diagnostic; it never compiles
the game as a fallback. The build log is `native/build.log`. The Mods page shows the running step
and lets you cancel it.

## Build refusals

| Error | What to do |
|---|---|
| `E_NATIVE_BUILD_SCRIPT` | Remove the mod's build script and ship its generated Rust source. `build = false` disables an incidental `build.rs`. Registry dependency build scripts are handled by the separate dependency permission prompt. |
| `E_NATIVE_FEATURES` | Use the pinned SDK's feature set; remove mod feature declarations and custom SDK feature requests. |
| `E_NATIVE_HOST` | Build on a host supported by the prebuilt SDK. |
| `E_MIXIN_STATE_LAYOUT` | Store per-object mod data in attachments instead of declaring `[[native.state]]`. |

Unavailable downloads can be retried. Missing SDKs, incompatible compiler identities and unsupported
transformations are refusals, not reasons to build the game locally.

## Mods made for another build

A native mod is made for one game build (its `[native] horizon` pin). On another build the
launcher compiles it when it fits: the mod uses only the mod API (no `[native.*]` table, no
internals), its package lists what it uses (`uses.json`, written by
`cargo horizon-mod build`), and every item on that list is unchanged in the build's
`surface.json`. The compile stays the final check: a mod that fits and still doesn't compile is
left out of that build and the others build.

A mod that doesn't fit stays installed and is left out of the build, with the changed items named
(at most five; the build log lists them all). A mod with a `[native.*]` table or internals, or a
package without the list, works on the build it was made for only.

When a newer release or Nightly is installed, the Mods page checks every native mod against it
before anything large is downloaded and says what an update would do: how many mods fit, and
which will be switched off and why. **Update** builds on the newer game; staying changes nothing.

When worlds of the instance were saved with a mod the build would leave out, the build stops and
names those worlds: opening them without the mod removes its blocks and what they hold, for good.
Staying on the current build is the default. The other choice backs those worlds up into the game
folder's `backups/` first (`native-build --back-up-worlds` on the command line); a backup that
fails stops the build with nothing changed.

## Where SDKs come from

SDK releases are read from this repository's releases. `HORIZON_SDK_BASE` replaces that base with
a mirror URL or a folder holding `sdk-<build id>/` folders (and the game releases' folders, for
their build ids), for example a copy for building offline. Every base is checked the same way,
against the release signature. Offline, a build whose SDK files are already kept works; one whose
files are missing says so, with the size to download. What a release holds and how it is checked:
[The mod SDK](../../reference/sdk/01-release.md); the fit rule in full:
[Fitting another build](../../reference/sdk/04-fit.md).
