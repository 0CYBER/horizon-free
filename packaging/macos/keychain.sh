#!/usr/bin/env bash
# The keychain the macOS builds are signed and notarized from: a keychain of its own (not the
# login keychain, which a runner's job can't unlock), holding only the "Developer ID
# Application" identity, its intermediate certificate and the notarytool profile. Its password
# is in 1Password (the Agents vault, read with the fleet's service account), so a job unlocks
# it without anyone typing anything. See packaging/macos/README.md.
#
#   packaging/macos/keychain.sh unlock         # a job's first macOS step: unlock, check, export the env
#   packaging/macos/keychain.sh lock           # its last
#   packaging/macos/keychain.sh create         # once: the keychain, with the password from 1Password
#   packaging/macos/keychain.sh import P12     # once, from the Mac's own Terminal: the identity
#   packaging/macos/keychain.sh notary         # once: the notarytool profile from the App Store Connect key
#
# MACOS_SIGN_IDENTITY names the identity (empty: nothing is signed, see package.sh);
# MACOS_SIGN_KEYCHAIN the keychain; MACOS_NOTARY_PROFILE the profile in it; MACOS_KEYCHAIN_PASSWORD,
# when set, is the keychain's password instead of 1Password's.
set -euo pipefail
kc=${MACOS_SIGN_KEYCHAIN:-$HOME/Library/Keychains/horizon-signing.keychain-db}
profile=${MACOS_NOTARY_PROFILE:-horizon-notary}
identity=${MACOS_SIGN_IDENTITY:-Developer ID Application: Ludovic Marie (YYJU63HSD4)}
# The 1Password CLI, signed in as the fleet's agents (`op` elsewhere), and the items it reads.
op=${OP:-$HOME/fleet/op}
[[ -x "$op" ]] || op=op
password_ref=${MACOS_KEYCHAIN_PASSWORD_REF:-op://Agents/Horizon signing keychain/password}
# "App Store Connect API - GitHub CI (DISPO)": a team key; by id, as the name's parentheses
# can't be in a secret reference.
notary_item=${MACOS_NOTARY_ITEM:-wwweyy32dwt6oylm6ypcfivsxe}

# The keychain's password: $MACOS_KEYCHAIN_PASSWORD (a CI secret elsewhere), or from 1Password.
password() {
  if [[ -n "${MACOS_KEYCHAIN_PASSWORD:-}" ]]; then
    printf '%s' "$MACOS_KEYCHAIN_PASSWORD"
  else
    "$op" read "$password_ref" < /dev/null
  fi
}

case "${1:-}" in
  unlock)
    security unlock-keychain -p "$(password)" "$kc"
    security find-identity -v -p codesigning "$kc" | grep -F "\"$identity\"" > /dev/null ||
      { echo "::error::$kc has no valid \"$identity\" (packaging/macos/keychain.sh import)"; exit 1; }
    xcrun notarytool history --keychain-profile "$profile" --keychain "$kc" > /dev/null ||
      { echo "::error::no notarytool profile $profile in $kc (packaging/macos/keychain.sh notary)"; exit 1; }
    echo "Unlocked $kc: \"$identity\" and the notarytool profile $profile"
    if [[ -n "${GITHUB_ENV:-}" ]]; then
      { echo "MACOS_SIGN_KEYCHAIN=$kc"; echo "MACOS_NOTARY_PROFILE=$profile"; } >> "$GITHUB_ENV"
    fi
    ;;
  lock)
    security lock-keychain "$kc"
    ;;
  create)
    [[ -e "$kc" ]] && { echo "$kc already exists"; exit 1; }
    security create-keychain -p "$(password)" "$kc"
    # Locks itself after two hours idle and when the Mac sleeps.
    security set-keychain-settings -t 7200 -l "$kc"
    echo "Created $kc"
    ;;
  import)
    p12=${2:?usage: keychain.sh import IDENTITY.p12}
    pass=$(password)
    security unlock-keychain -p "$pass" "$kc"
    read -rsp "The password you gave the .p12 when exporting it: " p12pass && echo
    security import "$p12" -k "$kc" -f pkcs12 -P "$p12pass" -T /usr/bin/codesign
    unset p12pass
    # The intermediate, so codesign builds the chain without the login keychain.
    issuer=$(security find-certificate -c "$identity" -p "$kc" | openssl x509 -noout -issuer |
      sed -n 's/.*CN *= *\([^,]*\).*/\1/p')
    if [[ -n "$issuer" ]] && ! security find-certificate -c "$issuer" "$kc" > /dev/null 2>&1 &&
      security find-certificate -c "$issuer" -p "$HOME/Library/Keychains/login.keychain-db" > "$kc.ca.pem"; then
      security import "$kc.ca.pem" -k "$kc"
    fi
    rm -f "$kc.ca.pem"
    # Only the one identity: anything else the .p12 held goes.
    security find-identity -v -p codesigning "$kc" | sed -n 's/^ *[0-9]*) \([0-9A-F]*\) "\(.*\)"$/\1 \2/p' |
      while read -r hash name; do
        [[ "$name" == "$identity" ]] || security delete-identity -Z "$hash" "$kc"
      done
    # codesign may use the key without asking.
    security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$pass" "$kc" > /dev/null
    rm -f "$p12"
    security find-identity -v -p codesigning "$kc"
    echo "Imported; $p12 is deleted."
    ;;
  notary)
    security unlock-keychain -p "$(password)" "$kc"
    key=$(mktemp) && chmod 600 "$key"
    trap 'rm -f "$key"' EXIT
    "$op" read "op://Agents/$notary_item/private_key_p8" < /dev/null > "$key"
    xcrun notarytool store-credentials "$profile" --key "$key" --keychain "$kc" \
      --key-id "$("$op" read "op://Agents/$notary_item/key_id" < /dev/null)" \
      --issuer "$("$op" read "op://Agents/$notary_item/issuer_id" < /dev/null)"
    ;;
  *)
    sed -n '/^set -e/q; 2,$s/^# \{0,1\}//p' "$0"
    exit 1
    ;;
esac
