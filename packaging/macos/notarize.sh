#!/usr/bin/env bash
# Notarizes signed macOS builds with Apple's notary service and staples the ticket to what can
# hold one: an .app (sent as a zip) and a .dmg are stapled, a .zip of bare programs can't be
# (Gatekeeper asks Apple online the first time one runs).
#
#   packaging/macos/notarize.sh FILE...
#
# Uses the notarytool profile $MACOS_NOTARY_PROFILE in the keychain $MACOS_SIGN_KEYCHAIN, which
# `packaging/macos/keychain.sh unlock` unlocks and exports.
set -euo pipefail
profile=${MACOS_NOTARY_PROFILE:?the notarytool profile (packaging/macos/keychain.sh unlock)}
keychain=()
[[ -n "${MACOS_SIGN_KEYCHAIN:-}" ]] && keychain=(--keychain "$MACOS_SIGN_KEYCHAIN")

for file in "$@"; do
  file=${file%/}
  sent=$file
  if [[ "$file" == *.app ]]; then
    sent=$(mktemp -d)/$(basename "$file").zip
    ditto -c -k --norsrc --noextattr --noqtn --keepParent "$file" "$sent"
  fi
  echo "Notarizing $(basename "$file")…"
  result=$(xcrun notarytool submit "$sent" --keychain-profile "$profile" ${keychain[@]+"${keychain[@]}"} \
    --wait --timeout 30m --output-format json)
  [[ "$sent" == "$file" ]] || rm -rf "$(dirname "$sent")"
  id=$(plutil -extract id raw -o - - <<< "$result")
  status=$(plutil -extract status raw -o - - <<< "$result")
  echo "$(basename "$file"): $status (submission $id)"
  if [[ "$status" != Accepted ]]; then
    xcrun notarytool log "$id" --keychain-profile "$profile" ${keychain[@]+"${keychain[@]}"} || true
    echo "::error::$(basename "$file") was not notarized: $status"
    exit 1
  fi
  case "$file" in
    *.app | *.dmg) xcrun stapler staple "$file" && xcrun stapler validate "$file" ;;
  esac
done
