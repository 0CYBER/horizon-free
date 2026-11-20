#!/usr/bin/env bash
# The vanilla data for a hosted runner (.github/workflows/parity-hosted.yml): datasets of the private repository
# horizon-rust/vanilla-data, the maintainers' own game files, encrypted, each unpacked into DESTINATION, a folder
# outside the checkout. Horizon's code never downloads the game; this reads only that repository.
#
#   VANILLA_DATA_DEPLOY_KEY=... VANILLA_DATA_PASSPHRASE=... bash tools/ci/fetch-vanilla-data.sh DESTINATION DATASET...
#
# The two secrets come from the environment of this one step. They are written to files in a private temporary folder
# and unset at once; that folder, with the key, the passphrase and the clone, is removed before the script ends,
# whatever happens. Only sizes, counts and timings are printed: never a file name from the archive, never a secret.
# DATASET/MANIFEST: "sha256 <digest of the encrypted whole>", then the chunk names in order.
#
# With VANILLA_DATA_CACHE (a folder outside the checkout that the workflow keeps in a cache entry), the datasets'
# encrypted chunks are taken from there when they match the MANIFESTs just read from the repository, which is what
# authenticates them: only the MANIFESTs' blobs are downloaded then. Otherwise the chunks are downloaded and the folder
# is refilled with them. Either way the script appends "cache=hit" or "cache=refreshed" to GITHUB_OUTPUT, and
# "digest=" with 16 hex digits of the MANIFESTs (of encrypted data: nothing of the game) for the entries' keys.
set -euo pipefail
umask 077
destination=${1:?destination}
shift
[ "$#" -gt 0 ] || { echo "::error::no dataset named"; exit 2; }
for dataset in "$@"; do
  [[ "$dataset" =~ ^[A-Za-z0-9._-]+$ ]] || { echo "::error::bad dataset name"; exit 2; }
done
: "${VANILLA_DATA_DEPLOY_KEY:?}" "${VANILLA_DATA_PASSPHRASE:?}"
case "$(realpath -m "$destination")/" in
  "$(realpath -m "${GITHUB_WORKSPACE:-$PWD}")"/*) echo "::error::the destination must be outside the checkout"; exit 2 ;;
esac

work=$(mktemp -d "${RUNNER_TEMP:-/tmp}/vanilla-fetch.XXXXXX")
cleanup() { rm -rf "$work"; }
trap cleanup EXIT
printf '%s\n' "${VANILLA_DATA_DEPLOY_KEY//$'\r'/}" > "$work/key"
printf '%s' "${VANILLA_DATA_PASSPHRASE//[$'\r\n']/}" > "$work/passphrase"
unset VANILLA_DATA_DEPLOY_KEY VANILLA_DATA_PASSPHRASE
ssh-keygen -y -f "$work/key" > /dev/null || { echo "::error::VANILLA_DATA_DEPLOY_KEY is not a private key"; exit 1; }
# github.com's ed25519 host key, pinned: its fingerprint is SHA256:+DiY3wvvV6TuJJhbpZisF/zLDA0zPMSvHdkr4UvCOqU, as
# GitHub publishes it (docs.github.com, "GitHub's SSH key fingerprints").
echo "github.com ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl" > "$work/known_hosts"
export GIT_SSH_COMMAND="ssh -i $work/key -o IdentitiesOnly=yes -o IdentityAgent=none -o UserKnownHostsFile=$work/known_hosts -o GlobalKnownHostsFile=/dev/null -o StrictHostKeyChecking=yes -o HostKeyAlgorithms=ssh-ed25519 -o BatchMode=yes"
export GIT_TERMINAL_PROMPT=0

started=$SECONDS
# Only these datasets' blobs: a shallow, blobless clone with a sparse checkout of their folders.
git clone -q --depth 1 --filter=blob:none --sparse git@github.com:horizon-rust/vanilla-data.git "$work/repo"
cache=${VANILLA_DATA_CACHE:-}
from=$work/repo
if [ -n "$cache" ]; then
  case "$(realpath -m "$cache")/" in
    "$(realpath -m "${GITHUB_WORKSPACE:-$PWD}")"/*) echo "::error::the cache folder must be outside the checkout"; exit 2 ;;
  esac
  # The MANIFESTs alone, then each cached dataset checked against the repository's.
  mkdir "$work/manifests"
  from=$cache
  for dataset in "$@"; do
    manifest="$work/manifests/$dataset"
    git -C "$work/repo" show "HEAD:$dataset/MANIFEST" > "$manifest" 2> /dev/null || { echo "::error::no dataset $dataset"; exit 1; }
    expected=$(sed -n 's/^sha256 \([0-9a-f]\{64\}\)$/\1/p' "$manifest")
    mapfile -t chunks < <(sed -n '2,$p' "$manifest")
    # Only plain files are read from the restored folder: a link, a pipe or a device there is never opened.
    plain=true
    [ -d "$cache/$dataset" ] && [ ! -L "$cache/$dataset" ] && [ ! -L "$cache" ] || plain=false
    for chunk in MANIFEST "${chunks[@]}"; do
      [[ "$chunk" =~ ^[A-Za-z0-9._-]+$ && -f "$cache/$dataset/$chunk" && ! -L "$cache/$dataset/$chunk" ]] || plain=false
    done
    if [ "$plain" != true ]; then from=$work/repo; continue; fi
    cached=$(cd "$cache/$dataset" 2> /dev/null && cat -- "${chunks[@]}" 2> /dev/null < /dev/null | sha256sum | cut -c1-64) || cached=
    cmp -s "$manifest" "$cache/$dataset/MANIFEST" && [ -n "$expected" ] && [ "$cached" = "$expected" ] || from=$work/repo
  done
fi
[ "$from" = "$cache" ] || git -C "$work/repo" sparse-checkout set "$@" > /dev/null
echo "clone $((SECONDS - started)) s ($([ "$from" = "$cache" ] && echo "chunks from the cache" || echo "chunks downloaded"))"
rm -rf "$destination" && mkdir -p "$destination"
for dataset in "$@"; do
  started=$SECONDS
  folder="$from/$dataset"
  [ -f "$folder/MANIFEST" ] || { echo "::error::no dataset $dataset"; exit 1; }
  expected=$(sed -n 's/^sha256 \([0-9a-f]\{64\}\)$/\1/p' "$folder/MANIFEST")
  mapfile -t chunks < <(sed -n '2,$p' "$folder/MANIFEST")
  for chunk in "${chunks[@]}"; do
    [[ "$chunk" =~ ^[A-Za-z0-9._-]+$ && -f "$folder/$chunk" ]] || { echo "::error::bad chunk in $dataset's manifest"; exit 1; }
  done
  found=$(cd "$folder" && cat -- "${chunks[@]}" | sha256sum | cut -c1-64)
  [ -n "$expected" ] && [ "$found" = "$expected" ] || { echo "::error::$dataset does not match its manifest"; exit 1; }
  encrypted=$(cd "$folder" && cat -- "${chunks[@]}" | wc -c)
  (cd "$folder" && cat -- "${chunks[@]}") |
    openssl enc -d -aes-256-cbc -pbkdf2 -iter 600000 -md sha256 -pass "file:$work/passphrase" |
    zstd -dq -T0 |
    tar -x --no-same-owner --no-same-permissions -C "$destination"
  echo "dataset $dataset: ${#chunks[@]} chunks, $encrypted bytes encrypted, verified and unpacked in" \
    "$((SECONDS - started)) s"
done
if [ -n "$cache" ]; then
  state=hit
  if [ "$from" != "$cache" ]; then
    state=refreshed
    rm -rf "$cache"
    for dataset in "$@"; do
      mkdir -p "$cache/$dataset"
      mapfile -t chunks < <(sed -n '2,$p' "$work/repo/$dataset/MANIFEST")
      (cd "$work/repo/$dataset" && cp -- MANIFEST "${chunks[@]}" "$cache/$dataset/")
    done
  fi
  digest=$(cd "$work/manifests" && cat -- "$@" | sha256sum | cut -c1-16)
  { echo "cache=$state"; echo "digest=$digest"; } >> "${GITHUB_OUTPUT:-/dev/null}"
fi
cleanup
trap - EXIT
echo "unpacked: $(du -sk "$destination" | cut -f1) KiB, $(find "$destination" -type f | wc -l) files"
