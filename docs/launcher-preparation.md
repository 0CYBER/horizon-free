# Launcher resource preparation

Release folders include `exporters/server`, `exporters/client` and an integrity manifest.
These contain only Horizon's original Java exporter classes. Minecraft jars, libraries,
assets and generated resources remain local and are verified against `research-lock.json`.
The same platform-independent class bundle is used in game and dedicated-server releases.

The release runner compiles the server and client source sets separately with
`javac --release <pinned major> -proc:none -encoding UTF-8`, resolving shared helpers once
per classpath. It checks every class and its signatures with `-Xverify:all` against the
pinned jars. The release job also compares two fresh isolated preparations with a fresh
batched preparation using a runtime-only Java home:

```sh
cargo build --release -p horizon-prepare
# Requires already imported Minecraft files in .local/minecraft/<version>.
target/release/horizon-prepare compile-exporters
# Copy server/, client/ and manifest.json from the printed folder into exporters/.
python3 tools/check_prepare_exporters.py --binary target/release/horizon-prepare
```

Development checkouts and changed sources compile on demand, into
`.local/exporters/<sha256>`. The key covers Java sources (including implicit helpers),
pinned dependency checksums and compiler settings. Class checksums detect missing or damaged
outputs. Compilation happens in a private staging folder before publishing a complete cache
entry. Sources and paths are normalized so one release bundle works on all three platforms.

Compatible exporters run sequentially through one server JVM and one client JVM. Both keep
Minecraft's initialized built-in registries. Every task restores the original registry tags
before replying; failures stop preparation. Collision exports share the first generated
registry and collision files, encoded block states and shape grids within the server worker.
Ordinary parity oracle processes keep their existing behavior.

Command-registry and creative-tab exporters remain separate processes: they install data
components as well as tags. The data generator also remains separate. Inspection of pinned
`net.minecraft.data.Main.main` showed that it calls `Util.shutdownExecutors` after running
its providers. Calling it inside a worker would shut down shared executors; incorporating
its providers directly needs a separate comparison of registry and executor lifetimes.
Fresh release preparation therefore starts five exporter/data-generator JVMs, down from 18,
and invokes no compiler. `HORIZON_PREPARE_ISOLATED=1` retains separate exporter JVMs for
output comparisons and diagnosis.

`tools/check_prepare_exporters.py` requires identical resource file sets and identical bytes
for every file stable across the two isolated runs. For the three known noisy JSON files,
it also requires semantic equality after normalizing object key order, opaque JVM lambda
addresses and creative-tab tag set order. Other variation fails the release check.

On Linux with Java 25, the output comparisons used 23,204 resource files per run.
All runs used precompiled exporters:

| Comparison | Isolated A | Isolated B | Batched |
|---|---:|---:|---:|
| Initial | 178.95 s | 104.25 s | 45.05 s |
| Repeat | 100.03 s | 176.06 s | 120.09 s |
| Final, immutable collision cache | 133.80 s | 123.79 s | 56.19 s |

These use fresh output directories with a warm OS file cache, on a machine with other
workloads. The first two rows include output hashing; the final row measures preparation
before comparison. The variability prevents treating these as a controlled speedup estimate.
The compiler saving is separate: release preparation removes the previously measured
15 compiler invocations (29–52 s).
