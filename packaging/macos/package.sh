#!/usr/bin/env bash
# Packages the macOS launcher: Horizon.app (the binary, Info.plist, the icon), signed, then the
# two release assets, horizon-launcher-macos-arm64.zip (what the launcher updates itself from;
# older launchers find the binary inside it) and horizon-launcher-macos-arm64.dmg (the installer:
# the app and an Applications link over a background, to drag one onto the other).
#
#   packaging/macos/package.sh BINARY VERSION OUTDIR
#
# Needs macOS (ditto, codesign, hdiutil) and dmgbuild (pip install dmgbuild==1.6.5; $DMGBUILD
# names it if it isn't on PATH). Signing: ad hoc by default, which needs no Apple account;
# set MACOS_SIGN_IDENTITY to a "Developer ID Application: …" identity in the keychain to sign
# for real (then notarize the .dmg: see packaging/macos/README.md).
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
binary=$1
version=$2
out=$(mkdir -p "$3" && cd "$3" && pwd)
app=$out/Horizon.app

rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$binary" "$app/Contents/MacOS/horizon-launcher"
chmod 755 "$app/Contents/MacOS/horizon-launcher"
cp "$here/../icons/horizon.icns" "$app/Contents/Resources/horizon.icns"
sed "s/@VERSION@/$version/g" "$here/Info.plist" > "$app/Contents/Info.plist"
printf 'APPL????' > "$app/Contents/PkgInfo"
plutil -lint "$app/Contents/Info.plist"

identity=${MACOS_SIGN_IDENTITY:--}
if [[ "$identity" == "-" ]]; then
  codesign --force --sign - "$app"
else
  codesign --force --options runtime --timestamp --sign "$identity" "$app"
fi
codesign --verify --strict --verbose=2 "$app"

rm -f "$out/horizon-launcher-macos-arm64.zip" "$out/horizon-launcher-macos-arm64.dmg"
# No resource forks or extended attributes: they would land as ._ files inside the bundle.
(cd "$out" && ditto -c -k --norsrc --noextattr --noqtn --keepParent Horizon.app horizon-launcher-macos-arm64.zip)

cat > "$out/dmg.json" <<JSON
{
  "title": "Horizon",
  "icon-size": 128,
  "background": "$here/background.png",
  "window": { "position": { "x": 200, "y": 160 }, "size": { "width": 660, "height": 420 } },
  "format": "UDZO",
  "contents": [
    { "x": 180, "y": 200, "type": "file", "path": "$app" },
    { "x": 480, "y": 200, "type": "link", "path": "/Applications" }
  ]
}
JSON
"${DMGBUILD:-dmgbuild}" -s "$out/dmg.json" Horizon "$out/horizon-launcher-macos-arm64.dmg"
rm "$out/dmg.json"
hdiutil verify "$out/horizon-launcher-macos-arm64.dmg" >/dev/null
echo "Packaged $out/horizon-launcher-macos-arm64.zip and .dmg"
