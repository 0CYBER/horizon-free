# The macOS launcher: Horizon.app in a .dmg

`package.sh BINARY VERSION OUTDIR` (run by `.github/workflows/launcher.yml` on macOS) makes:

- `Horizon.app`: `Contents/MacOS/horizon-launcher`, `Contents/Info.plist` (this folder's, bundle
  id `dev.horizon.launcher`, macOS 11 or later), `Contents/Resources/horizon.icns`
  (`packaging/icons`, from `cargo run -p xtask -- icons`).
- `horizon-launcher-macos-arm64.dmg`: the installer. Opening it shows the app and a link to
  Applications over `background.png` / `background@2x.png` (drawn from `background.html`); the
  player drags one onto the other. Built by [dmgbuild](https://github.com/dmgbuild/dmgbuild),
  which writes the Finder layout without a GUI session.
- `horizon-launcher-macos-arm64.zip`: the same app, zipped with `ditto`. The launcher updates
  itself from this: inside Horizon.app it replaces the whole bundle (copied beside it, then
  swapped in by renaming, then restarted); a bare `horizon-launcher` binary, as launchers before
  the app shipped, finds the binary inside the zip and replaces only itself.

## Signing and Gatekeeper

The app is signed **ad hoc** (`codesign --sign -`): no Apple account is needed, the bundle is
consistent and runs on Apple Silicon, but Gatekeeper doesn't know the developer. The first time
a player opens it after downloading:

- macOS 14 and earlier: right-click (or Control-click) Horizon in Applications, choose **Open**,
  then **Open** in the dialog.
- macOS 15 and later: open it once (it is refused), then **System Settings › Privacy & Security**,
  **Open Anyway** next to "Horizon was blocked", and confirm.

After that it opens normally, and its own updates don't ask again (they aren't downloaded by a
browser, so they carry no quarantine flag).

To sign and notarize for real (Ludovic's Apple Developer account): import the "Developer ID
Application" certificate into the runner's keychain, set `MACOS_SIGN_IDENTITY` to its name for
`package.sh` (it then signs with the hardened runtime and a timestamp), and after it, notarize and
staple the disk image:

```sh
xcrun notarytool submit dist/horizon-launcher-macos-arm64.dmg --keychain-profile horizon --wait
xcrun stapler staple dist/horizon-launcher-macos-arm64.dmg
```
