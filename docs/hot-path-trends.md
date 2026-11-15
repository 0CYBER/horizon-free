# Hot-path performance trends

The `hot-paths` workflow measures the game's hot paths when a maintainer dispatches it, on a machine that holds the
prepared resources, and prints the measurement in the run's summary. It measures; it never judges: nothing in it fails on a
slow result, it never runs on a pull request or on a schedule, and no contributor has to run it. It records nothing:
keep the results of two runs to compare them (below).

## What is measured

Each workload is fixed and seeded, built by the benchmark itself on a server `Game`
(`crates/horizon-client/src/game/runtime/hot_paths/`), in the release profile players get (ThinLTO). Every number
is divided by the workload's operations.

| Workload | What runs | One operation |
|---|---|---|
| `server.tick.empty` | the level's tick with nothing in it | a tick |
| `server.tick.mobs_128`, `mobs_2048` | the level's tick over cows, sheep, pigs and chickens on a stone floor | a mob's tick |
| `server.tick.hoppers_128`, `hoppers_2048` | the level's tick over rows of hoppers that hand items on | a hopper's tick |
| `command.selector_type_128` | `execute as @e[type=...] run data get entity @s Health`, parsed and run, 128 mobs | a command |
| `command.selector_nearest_128` | `execute at @e[type=...,sort=nearest,limit=4,distance=..48] if block ...` | a command |
| `chunk.save` | a chunk's snapshot with its block entities, its NBT, the bytes, zlib | a chunk |
| `chunk.load` | inflate, read the NBT, decode the chunk | a chunk |
| `mod.hook.no_listener` | the entity tick hook fired with no mod installed | a call |
| `mod.hook.native_listener` | the same with one native mod listening | a call |
| `mod.attachment.read`, `.write` | a typed attachment on 128 mobs, from inside the tick | a read or a write |

The two sizes of the tick workloads show a cost that grows faster than the count: the cost per mob or hopper at
2,048 against 128.

Three metrics per workload:

- **instructions**: the instructions the measured region executed, counted by Callgrind (`Ir`). It does not depend
  on the machine's speed or on what else runs there. This is the number to read a trend from.
- **allocations**: the calls into the allocator the region made, from the same Callgrind run.
- **wall_ns**: wall time without Valgrind. It depends on the machine, so the page compares it only between runs on the
  same kind of runner and CPU model, and it is noisy.

Every metric is sampled several times, each in a fresh process (3 samples for the counts, 5 for wall time), and a
record keeps every sample. The page shows a
metric's noise (the spread of the newest run's samples, or the difference between runs of one commit, whichever is
larger) and marks a metric whose noise is over 1% as noisy. It flags a step: a change against the run before of
more than 1% and more than three times the noise. A workload that changed (another checksum of its work, or
another operation count) starts a new series.

A series also starts again, with no step flagged, when the CPU model, the Rust version or the Valgrind version
changes: counts taken on either side of such a change are not compared. Whether instruction counts are the same on
two CPU models is not verified; the runs that measured the noise were on one.

Not measured yet: chunk meshing, the render frame, packet encoding and handling, a wasm listener and the wasm
boundary, a point or replaced function call, registry and tag lookups. Allocated bytes are not recorded, only the
number of allocator calls.

Most of a run is building the release executable. A follow-up is to measure an executable that main's own builds
already produce, so that a run costs only the measuring and can follow every merge.

## Run it yourself

It needs the prepared resources ([docs/parity-workflow.md](parity-workflow.md)) and, for the counts, Valgrind.

```sh
cargo build --release -p horizon-hot-paths-game
python3 tools/hot_paths.py measure --game target/release/horizon-hot-paths-game \
    --resources .local/horizon-client/26.3 --out hot-paths.json
python3 tools/hot_paths.py measure ... --only server.tick.mobs      # some workloads only
```

In CI, dispatch the workflow, with a commit to measure that commit: the result is the run's `hot-paths` artifact and
its summary.

To compare two commits, measure each and read the two tables; `record` and `page` build a page from any folder of
results:

```sh
python3 tools/hot_paths.py record hot-paths.json history && python3 tools/hot_paths.py page history
```

## Add a workload

Add it to a file under `hot_paths/`, or a new one called from `Game::hot_paths`: build the level with
`hot_level`, do the setup, then wrap the measured work in `recorder.measure(name, operations, || ...)` and return
from the closure something that shows the work was done (it becomes the checksum). Keep setup out of the closure,
keep it single-threaded and seeded, and check that the work really happened: the tick workloads fail when their
ticks change nothing. Then add its row to the table above.

## What a record holds

The commit, the date, the run number, the kind of runner and its CPU model, the kernel, the Rust and Valgrind versions, each
workload's operations and checksum, and the samples. `tools/hot_paths.py record` checks a result against that list
and keeps nothing else: no path, no file name, nothing read from the game's data.
