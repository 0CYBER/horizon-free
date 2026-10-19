#!/usr/bin/env bash
# Signs a release (docs/release-flow.md, "Signed checksums"): writes SHA256SUMS for the files given, then
# SHA256SUMS.sig beside them, one base64 Ed25519 signature per line over "horizon release <tag>\n" + SHA256SUMS,
# for every key set in RELEASE_SIGNING_KEY and RELEASE_SIGNING_KEY_NEXT (base64 PKCS#8, the `release`
# environment's secrets). The job that runs this holds the private keys, so it runs nothing from the repository
# but this file, which calls only the runner's sha256sum and OpenSSL; the publish job then checks the result
# with horizon-release-sign, as launchers do.
# EPHEMERAL=yes (dry runs) signs with a throwaway key instead and writes its public key to ephemeral.pub.
#   tools/ci/sign_release.sh v2026.10.19 dist/horizon-linux-x86_64.tar.gz dist/horizon-windows-x64.zip ...
set -euo pipefail
tag=$1; shift
dir=$(dirname "$1")
openssl=${OPENSSL:-openssl}
if command -v sha256sum > /dev/null; then sha=(sha256sum); else sha=(shasum -a 256); fi
umask 077
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

names=()
for file in "$@"; do
  [[ "$(dirname "$file")" == "$dir" ]] || { echo "$file: every file must be in $dir" >&2; exit 1; }
  names+=("$(basename "$file")")
done
(cd "$dir" && "${sha[@]}" -- "${names[@]}") > "$dir/SHA256SUMS"
{ printf 'horizon release %s\n' "$tag"; cat "$dir/SHA256SUMS"; } > "$work/message"
rm -f "$dir/SHA256SUMS.sig" "$dir/ephemeral.pub"

sign_with() {  # the key file in $1
  "$openssl" pkeyutl -sign -rawin -inkey "$1" -keyform DER -in "$work/message" -out "$work/signature"
  # base64 of the 64 bytes on one line (GNU base64 wraps at 76 characters).
  base64 < "$work/signature" | tr -d '\n' >> "$dir/SHA256SUMS.sig"
  echo >> "$dir/SHA256SUMS.sig"
  rm -f "$1" "$work/signature"
}

if [[ -n "${EPHEMERAL:-}" ]]; then
  "$openssl" genpkey -algorithm ed25519 -outform DER -out "$work/key"
  "$openssl" pkey -inform DER -in "$work/key" -pubout -outform DER | tail -c 32 | base64 > "$dir/ephemeral.pub"
  sign_with "$work/key"
else
  for name in RELEASE_SIGNING_KEY RELEASE_SIGNING_KEY_NEXT; do
    if [[ -n "${!name:-}" ]]; then
      # From the environment into a private temporary file: never an argument, never printed.
      printf '%s' "${!name}" | base64 --decode > "$work/key"
      sign_with "$work/key"
    fi
  done
fi
echo "$tag: SHA256SUMS lists ${#names[@]} files; $( [[ -f "$dir/SHA256SUMS.sig" ]] && echo "signed with $(wc -l < "$dir/SHA256SUMS.sig" | tr -d ' ') key(s)" || echo "no signing key, so unsigned")"
