# Parity workflow

How parity work is run, checked and merged. The harnesses themselves are described in [Rust rewrite status](rust-parity.md).

## Running the vanilla harnesses

`tools/parity.py` runs the `tools/test_*_parity.py` harnesses in parallel and reruns only what a change can affect:

```sh
python3 tools/parity.py              # harnesses whose inputs changed, or that never passed
python3 tools/parity.py food light   # these harnesses, whatever their state
python3 tools/parity.py --failed     # the ones that failed last time
python3 tools/parity.py --all        # everything, from scratch: the audit (26 minutes on the Mac mini alone)
python3 tools/parity.py --fresh food # don't reuse vanilla oracle results
python3 tools/parity.py --repeat 3 food   # run it three times; the runs must agree
python3 tools/parity.py --hosts macmini,thinkpad   # with other machines (default: tools/parity_hosts.json's default list, empty)
python3 tools/parity.py --list       # what would run
python3 tools/parity.py --status     # last result of each harness
python3 tools/parity.py --changed origin/main      # those a branch's changes can affect (CI's parity job)
python3 tools/parity.py --changed-files paths.txt  # the same for listed paths, in a copy without Git (tools/pr_ready.py)
```

Each harness runs unmodified under a tracer that records the files it depended on: its Python modules, its Java oracle and the oracle sources javac pulled in, other repository files it read, and the Rust sources of the probes it ran (from Cargo's `.d` dep-info files). A pass stands until one of those files, the pinned vanilla artifacts, `Cargo.lock` or the Rust/Java toolchains change. State and logs are in `.local/parity-runner/`; each harness's own outputs stay in its `.local/<name>/` directory.

Vanilla oracle runs are reused as well. `tools/parity_shim.py` stands in for `java` and `javac`: it records each oracle run's outputs and the content of every file under the paths its arguments name, and replays the run while those are unchanged. After a Rust-only change a harness pays only for its Rust side (`block_update`: 53 s to 5 s). Files an oracle writes outside its named paths aren't tracked, so a harness that fails while reusing oracle results is rerun fresh, and stops reusing them if it then passes. `--all` and `--fresh` never reuse them.

Probes build into `.local/parity-target/` with release optimisation but no LTO and 16 codegen units. On ludodesktop that rebuilds every `horizon-world` probe in 3 seconds after a one-line change, against 122 seconds with the release profile. Rust gives these settings identical results (no fast-math; overflow checks as in release), and the full harness suite passes under them. Probes that harnesses build in the dev profile get `opt-level = 3` too, keeping their overflow checks and debug assertions. Harnesses must locate binaries through `cargo metadata` or `lab.cargo_target()`, never a hard-coded `target/`.

The probes of `horizon-world` and of `horizon-client` are one binary per crate, `world_probe` and `client_probe` (`cargo build --release -p horizon-world --example world_probe`): a harness runs `world_probe <name> [arguments]`, and `<name>` is a module in `crates/<crate>/examples/probe/` (add it to the `mod` list and the `probes!` list in its `main.rs`; read arguments with `crate::args()` or `crate::args_os()`, which drop the name). Code several probes share goes in `examples/probe/support/` or a module of its own. Linking one binary instead of ~250 examples is most of what a probe build used to cost. Each crate's probe has its own target name (set in its `Cargo.toml`, `protocol_probe` too): Cargo copies every crate's examples into one `<profile>/examples/` directory, and with one shared name a harness could run the binary another harness had just copied there. The runner counts only the probes a harness ran, the probes and support modules they name, their own subdirectories and `dispatch.rs` as its dependencies, not `main.rs`, which only lists the probes: editing one probe reruns only its harnesses, and adding one reruns none.

Passes are shared between worktrees. A harness's key depends only on the content of its dependencies, not on where they are, so every pass is also written to `parity-shared/` beside the shared vanilla data (`PARITY_SHARED` overrides it). A worktree whose files match those a harness passed with, in any worktree, counts that pass without running the harness: after merging the integration branch, an agent's gate only runs the harnesses its own changes affect.

Every runner on the machine shares `PARITY_MACHINE_JOBS` slots (default: half the CPU threads), so several agents' runs together can't overload it. Parallel harnesses are also admitted by memory: each harness's recorded peak is reserved, within three quarters of the memory available at the start, and a harness starts only if the memory free at that moment still leaves 2 GB. `-j` caps the count here (default: `PARITY_LOCAL_JOBS`, else 3 while other machines help or a third of the CPU threads alone, within `$FLEET_LOAD_JOBS`).

### Harnesses that start the game

The game logs into `HORIZON_HOME` and falls back to the player's own game folder when it is unset, so a harness that starts the game, `horizon-ctl`, the launcher or the dedicated server builds the environment with `lab.game_env(**extra)`: no audio, and fresh scratch folders for `HORIZON_HOME`, `HORIZON_LAUNCHER_HOME`, `XDG_DATA_HOME` and `XDG_CONFIG_HOME`, removed when the harness exits (a folder you pass, or that the environment already has, is kept). The runner also gives each harness its own `HORIZON_HOME` under `.local/parity-runner/game-home/`. `tests/test_game_home.py` fails for a script in `tools/` that starts the game without it.

### Other machines

Harnesses can also run on the Mac mini and the ThinkPad (`tools/parity_hosts.json` describes them; its `default` list is empty, so pass `--hosts macmini,thinkpad` or set `PARITY_HOSTS`, after copying `shared-local/` to `~/code/horizon-parity/shared-local` on each host). The runner mirrors the worktree there with rsync (not `.git`, `.local`, `target` or ignored files) into `~/code/horizon-parity/remote/<host><path>`, whose `.local` links to the vanilla data in `~/code/horizon-parity/shared-local`, and starts `tools/parity.py --worker` over ssh. The worker builds the probes (about 80 s the first time) and runs what it is sent under the same tracer, admitting harnesses against its own free memory. One queue, longest first, feeds every machine, remote ones first: the Mac takes up to 14 harnesses, the desktop 3 and only those under 800 MB. Each pass comes back with the content hash of every file it depended on, so it is keyed on exactly what ran, and logs come back to `.local/parity-runner/logs/`; state, sharing and `gate.py` work as before. If a host can't be reached, or drops out, its harnesses run here.

Harnesses a host can't run are listed in the file: `local_only` (`save_roundtrip` runs the game on a virtual screen) and each host's `exclude`. The Mac excludes nothing: vanilla's `Math.sin`, `pow`, `log` and the like don't round the same on an Apple silicon Mac as on Linux (HotSpot uses fdlibm there, Intel's stubs on x86-64), and Rust's float functions round like neither (they call the platform libm), so code that mirrors a vanilla `Math` call uses `horizon_core::jmath`, which computes what HotSpot computes on the host architecture. The ThinkPad helps too, gently: Linux, 2 harnesses at `nice 19`, since it hosts the Claude account pool; it finds ALSA through `~/code/horizon-parity/alsa-dev` like the desktop.

### Shards and Rust scopes

A harness can declare, at the top level of its script:

- `PARITY_SHARDS = N`: it runs as `NAME#1` to `NAME#N`, in parallel, each told its share in `PARITY_SHARD=i/N` (`test_datafix_parity.py` upgrades every sixth world). `NAME` on the command line or in `--known-failures` means all of them.
- `PARITY_RUST_SCOPE = [...]`: the Rust sources its probes can reach, as path prefixes. Cargo's dep-info names every file of every crate a probe links, so without it a probe of `horizon-world` depends on all 350 of its files. The runner keeps only the scope's files if the scope is closed: nothing in it names a module, crate-root re-export or macro outside it (`crate::`, `super::`, other workspace crates; comments and strings ignored), declares a module it doesn't cover, or includes source, and nothing outside it implements anything for a type it defines. Crate roots then count without their declarations of other modules, so adding a module elsewhere doesn't rerun the harness. If the scope isn't closed the runner says why and keeps every file. `datafix` depends on 41 files this way, not 354.

## The gate

`tools/gate.py` is what a branch must pass before it is integrated: the public-tree check, rustfmt, clippy with warnings denied, the Rust and Python tests, then the harnesses whose inputs changed. It stops at the first failure and writes `.local/gate.json`. `--known-failures a,b` lets harnesses that already fail on the integration branch keep failing. `--quick` checks formatting, lints and Python tests only; `--no-parity` skips the harnesses.

The gate also looks for flaky harnesses: a harness whose script, oracle (Java under `mods/horizon/src/test`) or probe (a crate's example or binary) is new or changed since the merge base with `--flake-base` (default `parity-base`) must pass three runs in a row. The later runs reuse the first one's oracle results, so they cost only the Rust side, and run on the same machine; a harness whose runs disagree is reported `FLAKY` and fails the gate. A pass records how many runs it held over, so a later gate doesn't repeat it.

## Parallel work: worktrees and the integration branch

Parity work happens in one worktree and branch per area, next to an integration worktree that has the integration branch (`parity-base`, pushed as `t3/achieve-full-parity`) checked out. All worktrees share the vanilla data through `.local/` links to a `shared-local/` directory beside them.

```sh
python3 tools/supervise.py status                       # branches, progress, unpushed work, last gate
python3 tools/supervise.py new dispensers               # new worktree and branch from the integration branch
python3 tools/supervise.py save                         # commit and push unfinished work before agents stop
python3 tools/supervise.py check                        # which branches merge cleanly, alone and together
python3 tools/supervise.py integrate parity/dispensers  # merge, gate, push; undo the merge if the gate fails
python3 tools/supervise.py integrate --ready            # every parity/* branch with new commits, one gate
```

An agent working an area merges the integration branch first, uses `tools/parity.py` on its harnesses while it works, runs `tools/gate.py` before its last commit and pushes its branch. Integration happens only through `supervise.py integrate`, which runs the gate with the harnesses already failing on the integration branch as known failures: a branch may leave those failing, or fix them, but must not make any other harness fail.

`check` and `integrate` start by trial-merging the candidates with `git merge-tree`, in the object database, without touching any worktree: each branch alone, every pair, then the largest set without conflicts, merged in turn. That takes about a second for ten branches and names the conflicting files. `integrate` merges that set, gates it once (formatting, lints and tests before the long harness run; the flake check covers everything the branches change) and pushes. If the gate fails it tries each half of the set in turn, so the branches that pass still land; `--no-bisect` turns that off.

On ludodesktop, source `~/code/horizon-parity/env.sh` first: it sets the ALSA build path and the Cargo job cap. Debug builds carry line tables only (`[profile.dev]` in `Cargo.toml`), which keeps backtraces readable while keeping each worktree's build small; a fresh worktree builds `horizon-client` and its tests in under a minute.

## Reading vanilla

Besides the Vineflower sources in `.local/minecraft/<version>/sources/`, `tools/rea_vanilla.py` queries the shipped bytecode through REA (callers through lambdas, overloads, a second decompiler); see [decompilation status](decompilation-status.md#querying-the-shipped-bytecode-with-rea).

## Player-facing integration and deterministic fixtures

`python3 tools/lan_e2e.py` runs every LAN check with an integrated server thread,
the same path as normal single player. `python3 tools/test_friends.py --client
target/fast/horizon-client --relay target/fast/horizon-relay --baseline` covers
plain LAN, direct invitations, a local relay and ended invitations in that mode.
Both accept `--in-frame` for comparison and retain `--server-thread` for existing
callers. Run Linux games under `xvfb-run -a` when no display is available. Each
process gets scratch game, launcher, XDG data and config homes; telemetry, audio
and launcher self-update are disabled.

`lan_integration` and `friends_integration` register these suites with the normal
`parity.py` selection used by CI. Their traced imports and Cargo artifacts select
them when the drivers or game implementation change. LAN writes `results.json`
with the host mode and each result beside its screenshots and logs. A skipped
case is recorded explicitly and does not count as passing coverage.

The game-binary fixture audit keeps these complementary direct rule tests:

- `command_harness.rs`: command feedback, styled spans and result callbacks,
  held player position, explicit ticks and portal completion matching
  `VanillaCommandExecOracle`. LAN `commands`, `feedback`, `styles`, `levels` and
  `portals` cover command input, packet feedback and travel across the boundary.
- `dispense_fixture.rs`: seeded synthetic worlds, block/event traces, item
  use/mining and combat hooks matching `VanillaDispenserOracle`. These isolate
  rule results and RNG order; LAN machines, crafting, clicks, fireball, mining
  and combat exercise real input and server handling separately. The new
  `dispenser_input_host` / `dispenser_input_guest` checks press a button and
  verify authoritative inventory consumption and exactly one drop on both clients.
- `sculk_fixture.rs`: scripted events and exact per-tick sculk/warden traces
  matching `VanillaSculkOracle`; introducing wall-clock network ticks would
  destroy that deterministic oracle contract. `sculk_input_host` and
  `sculk_input_guest` separately exercise real footsteps reaching a sensor;
  failures remain visible rather than changing the oracle trace.
- `net_harness.rs`: already a real network client of a dedicated server; keep
  its handshake, world loading, chat and command packet assertions.

Library probes, `lan_bot`'s protocol hub fixture, generation/save tests and
streaming/mesh tests remain fast deterministic coverage. They do not become
full game launches. Setup and authoritative reads use the owning server;
interactions stay on the client. `server-hold on|off` provides a prediction-test
barrier between server ticks, including player packet handling, while transport
threads and control calls remain responsive. `stall MILLISECONDS` stalls only
the window to test recovery from a hung frame without process signals interrupting
socket reads. Neither mechanism changes normal gameplay.
