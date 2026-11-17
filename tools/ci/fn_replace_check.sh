#!/usr/bin/env bash
# Function replacement end to end (fn-replace): native mods' `[[native.mixin]]` rows (and API-only mods) built by the
# launcher's normal native build against a PREBUILT game, with no game rebuild, then run on a
# dedicated server.
#   [FNR_MODS="alpha beta"] [FNR_MOD_ROOT=DIR] [FNR_ROUTE=prebuilt|prebuilt-api] [FNR_TARGET=HOST] tools/ci/fn_replace_check.sh LAUNCHER SDK_BASE SCRATCH [RESOURCES]
# FNR_WASM_LAYOUT_PACKAGE=valid.hmod FNR_WASM_STD=std.wasm adds a wasm point-layout refusal check
# to the with-RESOURCES server run (not the build/link-only CI mode):
# its exported layout fingerprint is changed to zero, and its integrity hash is updated.
# SDK_BASE is a folder holding one `sdk-<commit>/` release folder (the SDK's archives, `sdk.json`, `SHA256SUMS` and its
# signature) of this checkout's HEAD: the launcher reads its SDK from there (HORIZON_SDK_BASE), checking the signature
# as it does a release's. A CI dry run signs its candidate SDK with a throwaway key and passes the public key in
# HORIZON_SDK_TRUSTED_KEY, which only a launcher built with debug_assertions trusts (horizon_release_sign::sdk_keys_for);
# a published release needs no key. With RESOURCES (a prepared `horizon-client/26.3` folder), the server starts, takes `/tick` commands
# and stops; every mixin helper prints a line starting `fnr:`.
set -euo pipefail
launcher=$(realpath "$1")
sdk_base=$(realpath "$2")
scratch=$3
resources=${4:-}
if [ -n "$resources" ]; then resources=$(realpath "$resources"); fi
side=${FNR_SIDE:-server}
case "$side" in server|client) ;; *) echo "FNR_SIDE must be server or client" >&2; exit 2;; esac
root=$(git rev-parse --show-toplevel)
# The game revision is the SDK's: its source is what a patched function is copied out of.
release=$(find "$sdk_base" -mindepth 1 -maxdepth 1 -type d -name 'sdk-*')
test "$(echo "$release" | wc -l)" = 1 && test -n "$release" || { echo "$sdk_base must hold exactly one sdk-<commit> folder" >&2; exit 2; }
revision=${release##*/sdk-}
mixins=$(sed -n 's/^pub const REV: &str = "\(.*\)";$/\1/p' "$root/crates/horizon-mod-package/src/mixins_tool.rs")

# FNR_KEEP=1 keeps the launcher's folder (toolchain, horizon-mixins, source): a warm build.
# FNR_MIXINS_FROM_SOURCE=1 builds horizon-mixins from its pinned source, dropping a kept tool: the
# launcher otherwise downloads the prebuilt one, and its source build has to stay proven.
if [ -n "${FNR_KEEP:-}" ]; then rm -rf "$scratch/mods" "$scratch/engine" "$scratch/run" "$scratch/swap-mods" "$scratch/swap-engine" "$scratch/launcher/native/server-builds" "$scratch/launcher/versions"/native-*; else rm -rf "$scratch"; fi
mkdir -p "$scratch"/{home,launcher,data,config,trash,clipboard,desktop,mods}
scratch=$(realpath "$scratch")
data=$scratch/launcher

# The pinned source a patched function is copied out of, from this checkout.
source=$data/native/source/$revision
rm -rf "$source"
mkdir -p "$source"
git -C "$root" archive "$revision" | tar -x -C "$source"
find "$source" -type l -delete
printf '%s' "$revision" > "$source/.horizon-revision"

base=$data/versions/ci
mkdir -p "$base/.local/horizon-client/26.3"
touch "$base/.installed"
echo '{}' > "$base/.local/horizon-client/26.3/horizon-resources.json"
version=$data/versions/native-ci
mkdir -p "$version"
touch "$version/.installed"
# On Windows the launcher picks the build target from the installed game's PE machine; any x64
# executable stands in for it, and the launcher itself is one.
case $launcher in *.exe) cp "$launcher" "$version/horizon-client.exe" ;; esac
printf '{"base": "ci", "horizon": "%s", "target": "%s"}\n' "$revision" "$side" > "$version/native-mods.json"

mods=$scratch/mods/fnr
mkdir -p "$mods"
for mod in ${FNR_MODS:-alpha beta}; do
  cp -r "${FNR_MOD_ROOT:-$root/tools/ci/fn-replace-check}/$mod" "$mods/fnr_$mod"
  sed -i "s/^horizon = \"HEAD\"$/horizon = \"$revision\"/; s/^mixins = \"MIXINS\"$/mixins = \"$mixins\"/" "$mods/fnr_$mod/horizon-mod.toml"
done
# The game library (libhorizon_mod_host.so / .dylib, or horizon_mod_host.dll on Windows): its SHA-256 in the SDK's
# runtime archive for this side, which the unpacked copy must still match once the mods are built.
game_hash=$(python3 - "$release" "$side" <<'PYGAME'
import hashlib, os, sys, tarfile
from pathlib import Path
release, side = Path(sys.argv[1]), sys.argv[2]
archives = [a for a in release.glob("runtime-*.tar.xz") if a.name.endswith("-server.tar.xz") == (side == "server")]
target = os.environ.get("FNR_TARGET", "")
if target:
    suffix = "-server" if side == "server" else ""
    archives = [a for a in archives if a.name == f"runtime-{target}{suffix}.tar.xz"]
assert len(archives) == 1, f"one runtime archive for {target or 'the host'}/{side}, found {[a.name for a in archives]}; set FNR_TARGET when using a complete SDK"
with tarfile.open(archives[0], "r:xz") as tar:
    names = [m for m in tar.getmembers() if m.isfile() and Path(m.name).name.endswith(("horizon_mod_host.so", "horizon_mod_host.dylib", "horizon_mod_host.dll"))]
    assert len(names) == 1, f"one game library, found {[m.name for m in names]}"
    print(hashlib.sha256(tar.extractfile(names[0]).read()).hexdigest())
PYGAME
)
game_library() { find "$data/native/prebuilt/$revision"-* -type f \( -name '*horizon_mod_host.so' -o -name '*horizon_mod_host.dylib' -o -name '*horizon_mod_host.dll' \) | head -1; }

env_vars=(HORIZON_HOME="$scratch/home" HORIZON_LAUNCHER_HOME="$data" XDG_DATA_HOME="$scratch/data"
  XDG_CONFIG_HOME="$scratch/config" HORIZON_LAUNCHER_NO_SELF_UPDATE=1 HORIZON_NO_AUDIO=1
  HORIZON_LAUNCHER_TRASH="$scratch/trash" HORIZON_LAUNCHER_CLIPBOARD="$scratch/clipboard"
  HORIZON_LAUNCHER_DESKTOP="$scratch/desktop" HORIZON_NATIVE_CARGO_TIMINGS=1 HORIZON_SDK_BASE="$sdk_base")
started=$(date +%s.%N)
output_args=()
if [ -n "${FNR_MIXINS_FROM_SOURCE:-}" ]; then
  rm -rf "$data/native/tools"
  output_args+=(--mixins-from-source)
fi
if [ "$side" = server ]; then output_args+=(--server-output "$scratch/engine"); fi
env "${env_vars[@]}" "$launcher" native-build "$mods" "${output_args[@]}" --version native-ci --profile optimized \
  --trust-native-code > "$scratch/native-build.out" 2> "$scratch/native-build.err" || built=$?
cat "$scratch/native-build.out"
cat "$scratch/native-build.err" >&2
test -z "${built:-}" || exit "$built"
# The launcher's plan for this build, an instance's or a server folder's export alike: the prebuilt game.
grep -F 'Prebuilt game: only the mods compile' "$scratch/native-build.err"
if [ "$side" = client ]; then
  binary=$(find "$data/versions" -type f \( -name horizon-client -o -name horizon-client.exe \) ! -path "$version/*" | head -1)
  test -n "$binary"
  cp -a "$(dirname "$binary")" "$scratch/engine"
  case "$binary" in
    *.exe) cp "$scratch/engine/horizon-client.exe" "$scratch/engine/horizon-engine.exe" ;;
    *) cp "$scratch/engine/horizon-client" "$scratch/engine/horizon-engine" ;;
  esac
fi
echo "native build: $(python3 -c "print(round($(date +%s.%N) - $started, 1))") s"
test -x "$scratch/engine/horizon-engine" || test -x "$scratch/engine/horizon-engine.exe"
python3 - "$scratch/engine" <<'PYMARK'
import pathlib, sys
engine = pathlib.Path(sys.argv[1])
binary = next(p for p in (engine / "horizon-engine", engine / "horizon-engine.exe") if p.is_file())
assert b"horizon-private-holder-v2;" in binary.read_bytes(), f"holder capability missing from {binary}"
PYMARK
if [ "$side" = server ]; then
  # The server folder's export: the launcher says where the engine is, and installs no instance build.
  grep -F "Built the server engine: $scratch/engine/horizon-engine" "$scratch/native-build.out"
  test -z "$(find "$data/versions" -type f -name 'horizon-client*' ! -path "$version/*")"
fi
log=$data/native/build.log
# Where the build's time went: each command's time and the whole prebuilt compile.
grep -E ': [0-9]+ ms$|^Mods compiled against the prebuilt game in ' "$log" || true
echo "Runner load: $(cut -d' ' -f1-3 /proc/loadavg 2>/dev/null) on $(nproc 2>/dev/null) CPUs"
# What the kept build folders weigh on the Storage page's Build files row.
for kept in "$data"/native/prebuilt-build/*-*/; do
  [ -d "$kept" ] && echo "Kept build folder $(basename "$kept"): $(du -sk "$kept" | cut -f1) kB"
done
# The registry route's one cargo command, split by crate (cargo's own timings), longest first.
timings=$(find "$data/native" -path '*cargo-timings/cargo-timing.html' 2>/dev/null | head -1)
if [ -n "$timings" ]; then
  python3 - "$timings" <<'PY'
import json, re, sys
text = open(sys.argv[1], encoding="utf-8").read()
match = re.search(r"UNIT_DATA = (\[.*?\]);", text, re.S)
units = json.loads(match.group(1)) if match else []
for unit in sorted(units, key=lambda u: -u.get("duration", 0)):
    label = f"{unit['name']}{unit.get('target', '')}".replace('"', "")
    print(f"cargo unit {label}: {round(unit.get('duration', 0) * 1000)} ms")
PY
fi
# Only a `[[native.*]]` row patches a function; a mod of hooks alone must not say it did.
if grep -qs '^\[\[native\.' "$mods"/*/horizon-mod.toml; then
  grep -F 'Function replacement:' "$log"
elif grep -sF 'Function replacement:' "$log"; then
  echo "a set with no [[native.*]] row logged a function replacement"; exit 1
fi
# The prebuilt route, and no game source compiled. FNR_ROUTE names the route the whole set must
# take: `prebuilt-api` (every mod API-only) or `prebuilt` (a mod with a `[native]` table row).
grep -F '"route": "prebuilt' "$data"/versions/*/native-mods.json > /dev/null || grep -rF 'prebuilt' "$log" > /dev/null
if [ -n "${FNR_ROUTE:-}" ]; then grep -F "Native mods route: $FNR_ROUTE." "$log"; fi
# The API route plays the package's own executable: it carries the holder marker in its own bytes.
if [ "${FNR_ROUTE:-}" = prebuilt-api ]; then
  marker=$(sed -n 's/^pub static CAPABILITY: \[u8; [0-9]*\] = \*b"\([^"]*\)";$/\1/p' "$root/crates/horizon-mod-package/src/holder.rs")
  test -n "$marker"
  # Whichever of the two exists: `ls` of the missing one exits 2, which pipefail would make the step's exit.
  for played in "$scratch/engine/horizon-engine" "$scratch/engine/horizon-engine.exe"; do
    [ -f "$played" ] && break
  done
  grep -qaF "$marker" "$played" || { echo "$played does not carry the holder marker $marker"; exit 1; }
  echo "holder marker $marker found in $played"
fi
test ! -d "$data/native/stage/crates/horizon-client" || { echo "the game was staged for a source build"; exit 1; }
test "$(sha256sum "$(game_library)" | cut -d' ' -f1)" = "$game_hash"

if [ -n "$resources" ]; then
  run=$scratch/run
  mkdir -p "$run"
  echo eula=true > "$run/eula.txt"
  layout_mod=
  if [ -n "${FNR_WASM_LAYOUT_PACKAGE:-}" ]; then
    cp "${FNR_WASM_STD:?set FNR_WASM_STD to the matching horizon-wasm-std.wasm}" "$scratch/engine/horizon-wasm-std.wasm"
    layout_mod=$(python3 "$root/tools/ci/mismatch_wasm_layout.py" "$FNR_WASM_LAYOUT_PACKAGE" "$run/horizon-mods/layout-refused.hmod")
    printf 'mod.%s:true\n' "$layout_mod" > "$run/horizon-mods.txt"
  fi
  # Compile a changed copy of one mod, then swap only its library into the first build.
  # Its public probe adds compiled code without changing the hooks the check asserts below.
  executable_hash=$(sha256sum "$scratch/engine/horizon-engine" | cut -d' ' -f1)
  swap_mods=$scratch/swap-mods
  cp -a "$mods" "$swap_mods"
  python3 - "$scratch/engine/native-mods.json" "$swap_mods" <<'PYCHANGE'
import json, pathlib, sys, tomllib
record = json.loads(pathlib.Path(sys.argv[1]).read_text())
mod_id = next(r["mod"] for r in record["libraries"] if r["kind"] == "mod")
for folder in pathlib.Path(sys.argv[2]).iterdir():
    manifest = folder / "horizon-mod.toml"
    if manifest.is_file() and tomllib.loads(manifest.read_text())["id"] == mod_id:
        cargo = tomllib.loads((folder / "Cargo.toml").read_text())
        source = folder / cargo.get("lib", {}).get("path", "src/lib.rs")
        with source.open("a") as stream:
            stream.write("\n#[inline(never)] pub fn __horizon_library_swap_probe() -> u64 { 0x4041 }\n")
        break
else:
    raise SystemExit(f"no source for swap mod {mod_id}")
PYCHANGE
  swap_args=()
  if [ "$side" = server ]; then swap_args+=(--server-output "$scratch/swap-engine"); fi
  env "${env_vars[@]}" "$launcher" native-build "$swap_mods" "${swap_args[@]}" --version native-ci --profile optimized \
    --trust-native-code > "$scratch/swap-build.out" 2> "$scratch/swap-build.err" || {
      cat "$scratch/swap-build.out"; cat "$scratch/swap-build.err" >&2; exit 1;
    }
  python3 - "$scratch/engine" "$scratch/swap-engine" "$data/versions" <<'PYLIB'
import hashlib, json, pathlib, shutil, sys
folder = pathlib.Path(sys.argv[1])
record = folder / "native-mods.json"
rows = json.loads(record.read_text())
row = next(r for r in rows["libraries"] if r["kind"] == "mod")
library = folder / row["file"]
original = library.read_bytes()
for root in map(pathlib.Path, sys.argv[2:]):
    for candidate in root.rglob("native-mods.json"):
        build = json.loads(candidate.read_text())
        for replacement in build.get("libraries", []):
            if replacement["mod"] != row["mod"] or replacement["identity"] != row["identity"]:
                continue
            path = candidate.parent / replacement["file"]
            if path.read_bytes() == original:
                continue
            shutil.copyfile(path, library)
            row["sha256"] = hashlib.sha256(library.read_bytes()).hexdigest()
            record.write_text(json.dumps(rows))
            print(f"Swapped independently rebuilt library for {row['mod']}")
            raise SystemExit(0)
raise SystemExit("no changed library with the expected identity was built")
PYLIB
  test "$(sha256sum "$scratch/engine/horizon-engine" | cut -d' ' -f1)" = "$executable_hash"
  # The start below loads the swapped library: the loader hashes every library against the record
  # before loading any, and the record now holds only the new bytes, so a start that passes the
  # checks ran the rebuilt library, with the executable unchanged.
  # A loop mod gets a forced chunk of hoppers handing stone along each row, so the loop visits them,
  # and a row of each loop mod's counters beside them.
  hoppers=
  if [[ " ${FNR_MODS:-alpha beta} " == *" loop"* ]]; then
    hoppers='forceload add 0 0
fill 0 100 0 9 100 9 minecraft:hopper[facing=east]{Items:[{Slot:0b,id:"minecraft:stone",count:64}]}'
  fi
  if [[ " ${FNR_MODS:-alpha beta} " == *" loop "* ]]; then
    hoppers+=$'\nfill 0 100 11 9 100 11 fnr_loop:counter'
  fi
  if [[ " ${FNR_MODS:-alpha beta} " == *" loop_second "* ]]; then
    hoppers+=$'\nfill 0 100 12 9 100 12 fnr_loop_second:counter'
  fi
  (sleep 30; echo "$hoppers"; sleep 5; echo "tick rate 40"; sleep 3; echo "tick rate 13"; sleep 3; echo "tick query"; sleep 3; echo "stop"; sleep 20) |
    (cd "$run" && env "${env_vars[@]}" timeout 180 "$scratch/engine/horizon-engine" --resources "$(realpath "$resources")" \
      --dedicated --nogui) > "$scratch/server.log" 2>&1 || true
  grep -F 'fnr:' "$scratch/server.log"
  if [ -n "$layout_mod" ]; then
    grep -F "wasm mod $layout_mod refused: built for another version of the mod points API (mod layout 0, game layout " "$scratch/server.log"
  fi
  if [[ " ${FNR_MODS:-alpha beta} " == *" alpha "* ]]; then
    grep -F 'OVERWRITE by fnr_alpha' "$scratch/server.log"
  fi
  if [[ " ${FNR_MODS:-alpha beta} " == *" loop "* ]]; then
    slots=2
    if [[ " ${FNR_MODS:-alpha beta} " == *" loop_second "* ]]; then
      slots=3
      # Two mods' block entities in one world: the second mod's slot counted its own too.
      grep -F 'fnr: loop_second own #1: fnr_loop_second:counter' "$scratch/server.log"
    fi
    # Each slot checks every block entity it took for its own, and never took another mod's.
    if grep -F 'own FOREIGN at' "$scratch/server.log"; then
      echo "a loop slot took another mod's block entity for its own"; exit 1
    fi
    # Both slots ran in priority order, the count picked out its mod's counters, the throttle wrote
    # cooldowns, and the entry's HEAD composed.
    grep -F 'fnr: loop count before #1:' "$scratch/server.log"
    grep -F 'fnr: loop count after #1:' "$scratch/server.log"
    grep -F 'fnr: loop count own #1: fnr_loop:counter' "$scratch/server.log"
    grep -F 'fnr: loop throttle skips #1:' "$scratch/server.log"
    grep -F 'fnr: loop HEAD tick_block_entities' "$scratch/server.log"
    # A call-site row inside the loop body ran in the loop's copy, and an API listener at the entry
    # fired through the same cell as the loop.
    grep -F 'fnr: loop MODIFY_EXPRESSION_VALUE chunk check inside the loop' "$scratch/server.log"
    grep -F 'fnr: loop api head listener at tick_block_entities' "$scratch/server.log"
    grep -F "Published loops: 1 filled by $slots mod slots" "$log"
    # The slot ran the loop again from inside it: the nested run was the game's own, logged once.
    grep -F 'fnr: loop count ran the loop again inside it: true' "$scratch/server.log"
    test "$(grep -cF 'Published loop block_entities.tick: it was entered again while it ran' "$scratch/server.log")" = 1
  fi
  if [[ " ${FNR_MODS:-alpha beta} " == *" loop_panic "* ]]; then
    # The slot's panic is the server's crash, and its report names the mod and the loop.
    grep -F 'in native mod fnr_loop_panic, hook loop block_entities.tick, part fnr_loop_panic::Tenth' "$run"/crash-reports/*.txt
    grep -F 'Saving chunks for level' "$scratch/server.log"
  fi
  if [[ " ${FNR_MODS:-alpha beta} " == *" api_only "* ]]; then
    grep -F 'fnr: api_only server_started, sin 0.479' "$scratch/server.log"
    grep -F 'fnr: api_only listener at tick_block_entities' "$scratch/server.log"
  fi
  if [[ " ${FNR_MODS:-alpha beta} " == *" fdshape "* ]]; then
    grep -F 'fnr: native_server_started dedicated=true' "$scratch/server.log"
  fi
  test "$(sha256sum "$scratch/engine/horizon-engine" | cut -d' ' -f1)" = "$executable_hash"
  corrupt_mod=$(python3 - "$scratch/engine" <<'PYLIB'
import json, pathlib, sys
folder = pathlib.Path(sys.argv[1])
row = next(r for r in json.loads((folder / "native-mods.json").read_text())["libraries"] if r["kind"] == "mod")
with (folder / row["file"]).open("ab") as stream:
    stream.write(b"corrupted-library")
print(row["mod"])
PYLIB
)
  corrupt_status=0
  (cd "$run" && env "${env_vars[@]}" timeout 30 "$scratch/engine/horizon-engine" --resources "$(realpath "$resources")" --dedicated --nogui) > "$scratch/corrupt.log" 2>&1 || corrupt_status=$?
  if (( corrupt_status == 0 || corrupt_status >= 124 )); then
    echo "corrupted native library was accepted"; exit 1
  fi
  grep -E "^Horizon: refusing native startup: mod ${corrupt_mod}: library [^/]+ hash mismatch " "$scratch/corrupt.log"
  if grep -F 'fnr:' "$scratch/corrupt.log"; then
    echo "mod code ran before library hashes passed"; exit 1
  fi

fi
