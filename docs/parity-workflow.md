# Parity workflow

How parity work is run, checked and merged. What is ported and where it is checked: [Rust rewrite status](rust-parity.md).

## Datafix downloads and author self-checks

Datafix generation owns a private writable root, defaulting to checkout `.local/datafix`.
`HORIZON_DATAFIX_ROOT` overrides it (relative values resolve from the invoking working directory).
Minecraft input symlinks never select this output root. The tool refuses shared vanilla
roots and their ancestors, escaping default symlinks, escaping descendant symlinks, file roots and unwritable
ancestors before downloads, world generation or `generate.lock` creation. Choose a new private
directory on refusal; never change permissions on shared fixtures to bypass the guard.

Author `pr_ready` excludes datafix by default and prints one opt-in reminder. To explicitly
allow it among the affected harnesses, use a private directory. Local opt-in preflights the
root conservatively even when affected dependencies are unknown, and passes its validated
absolute path into subprocesses so a working-directory change cannot reinterpret it:

```sh
HORIZON_DATAFIX_ROOT="$PWD/.local/datafix-private" python3 tools/pr_ready.py --datafix
```

Direct `tools/parity.py datafix` is also an explicit harness run and may download jars and
runtimes, launch servers and generate worlds. Metadata for cold Linux 1.8.9 alone lists
8,320,755 bytes for the server jar plus 235,129,362 bytes for the pinned legacy runtime:
243,450,117 bytes before JSON metadata and generated worlds. Other versions/runtimes add
more; cached payload can be zero. No runtime measurement is available for this repair.
Fork issue arismoko/horizon#3027 tracks whether CI runs datafix.

## Running the vanilla harnesses

Same-machine fixtures created with `lab.game_env` use `HORIZON_TEST_LOOPBACK=1`: Horizon
listens on loopback and does not scan or announce LAN worlds. This avoids Windows Firewall
permission prompts for newly compiled fixture executables. Set `HORIZON_TEST_LOOPBACK=0`
explicitly when testing real network interfaces or multicast discovery. Local vanilla server
fixtures also set `server-ip=127.0.0.1`; use one installed JDK via `JAVA_HOME` and its `bin`
on `PATH`, rather than copying Java to each fixture directory. Keep the firewall enabled.

`tools/parity.py` runs the `tools/test_*_parity.py` harnesses in parallel and reruns only what a change can affect:

```sh
python3 tools/parity.py              # harnesses whose inputs changed, or that never passed
python3 tools/parity.py food light   # these harnesses, whatever their state
python3 tools/parity.py --failed     # the ones that failed last time
python3 tools/parity.py --all        # everything, from scratch: the audit (26 minutes on the Mac mini alone)
python3 tools/parity.py --fresh food # don't reuse vanilla oracle results
python3 tools/parity.py --repeat 3 food   # run it three times; the runs must agree
python3 tools/parity.py --hosts macmini,playhorizon   # with other machines (default: tools/parity_hosts.json's default list, empty)
python3 tools/parity.py --list       # what would run
python3 tools/parity.py --status     # last result of each harness
python3 tools/parity.py --changed origin/main      # those a branch's changes can affect (CI's parity job)
python3 tools/parity.py --changed-files paths.txt  # the same for listed paths, in a copy without Git (tools/pr_ready.py)
```

On CI, a pull request runs the parity job only when it changes a harness, an oracle, a probe or the runner; everything else is checked on main by `parity-main`, a batch of merges at a time; the merge that broke a harness is found by hand with `tools/parity_main.py bisect` ([release-flow.md](release-flow.md#parity-on-main)). Game releases wait for it.

Each harness runs unmodified under a tracer that records the files it depended on: its Python modules, its Java oracle and the oracle sources javac pulled in, other repository files it read, and the Rust sources of the probes it ran (from Cargo's `.d` dep-info files). A pass stands until one of those files, the pinned vanilla artifacts, `Cargo.lock` or the Rust/Java toolchains change. State and logs are in `.local/parity-runner/`; each harness's own outputs stay in its `.local/<name>/` directory.

Vanilla oracle runs are reused as well. `tools/parity_shim.py` stands in for `java` and `javac`: it records each oracle run's outputs and the content of every file under the paths its arguments name, and replays the run while those are unchanged. After a Rust-only change a harness pays only for its Rust side (`block_update`: 53 s to 5 s). Files an oracle writes outside its named paths aren't tracked, so a harness that fails while reusing oracle results is rerun fresh, and stops reusing them if it then passes. `--all` and `--fresh` never reuse them.

Probes build into `.local/parity-target/` with release optimisation but no LTO and 16 codegen units. On a 24-thread Linux machine that rebuilds every `horizon-world` probe in 3 seconds after a one-line change, against 122 seconds with the release profile. Rust gives these settings identical results (no fast-math; overflow checks as in release), and the full harness suite passes under them. Probes that harnesses build in the dev profile get `opt-level = 3` too, keeping their overflow checks and debug assertions. Harnesses must locate binaries through `cargo metadata` or `lab.cargo_target()`, never a hard-coded `target/`.

The probes of `horizon-world` and of `horizon-client` are one binary per crate, `world_probe` and `client_probe` (`cargo build --profile fast -p horizon-world --example world_probe`): a harness runs `world_probe <name> [arguments]`, and `<name>` is a module in `crates/<crate>/examples/probe/` (add it to the `mod` list and the `probes!` list in its `main.rs`; read arguments with `crate::args()` or `crate::args_os()`, which drop the name). Code several probes share goes in `examples/probe/support/` or a module of its own. Linking one binary instead of ~250 examples is most of what a probe build used to cost. Each crate's probe has its own target name (set in its `Cargo.toml`, `protocol_probe` too): Cargo copies every crate's examples into one `<profile>/examples/` directory, and with one shared name a harness could run the binary another harness had just copied there. The runner counts only the probes a harness ran, the probes and support modules they name, their own subdirectories and `dispatch.rs` as its dependencies, not `main.rs`, which only lists the probes: editing one probe reruns only its harnesses, and adding one reruns none.

Passes are shared between worktrees. A harness's key depends only on the content of its dependencies, not on where they are, so every pass is also written to `parity-shared/` beside the shared vanilla data (`PARITY_SHARED` overrides it). A worktree whose files match those a harness passed with, in any worktree, counts that pass without running the harness: after merging the integration branch, an agent's gate only runs the harnesses its own changes affect.

All runners on a machine share `PARITY_MACHINE_JOBS` slots (default: half the CPU threads), so several agents' runs together can't overload it. Parallel harnesses are also admitted by memory: each harness's recorded peak is reserved, within three quarters of the memory available at the start, and a harness starts only if the memory free at that moment still leaves 2 GB. `-j` caps the count here (default: `PARITY_LOCAL_JOBS`, else 3 while other machines help or a third of the CPU threads alone, within `$FLEET_LOAD_JOBS`).

### Harnesses that start the game

The game logs into `HORIZON_HOME` and falls back to the player's own game folder when it is unset, so a harness that starts the game, `horizon-ctl`, the launcher or the dedicated server builds the environment with `lab.game_env(**extra)`: no audio, and fresh scratch folders for `HORIZON_HOME`, `HORIZON_LAUNCHER_HOME`, `XDG_DATA_HOME` and `XDG_CONFIG_HOME`, removed when the harness exits (a folder you pass, or that the environment already has, is kept). The runner also gives each harness its own `HORIZON_HOME` under `.local/parity-runner/game-home/`. `tests/test_game_home.py` fails for a script in `tools/` that starts the game without it.

`frame_identity` (`tools/test_frame_identity_parity.py`) holds the picture where a change shouldn't move it, as the HDR design note's first tests ask ([design/hdr-linear-light.md](design/hdr-linear-light.md#tests)). Its oracle is the base build, not vanilla: pictures of the game hold Minecraft's textures and can't be committed. It builds `BASE_SHA` (else the merge-base with `origin/main`) in a worktree and target folder of its own under `.local/frame-identity/`, kept between runs so a later base rebuilds incrementally, draws fixed scenes offscreen with each build under `HORIZON_STILL=1` (HDR off; the Golden Hour pack in SDR and with offscreen HDR output; HDR turned off in a running game), and requires identical pixels, writing `diff-<scene>.png` where they aren't. Both builds draw on lavapipe where Mesa's Vulkan driver is installed, else on the machine's default adapter. When the change touches only `tools/`, `tests/`, `.github/` or Markdown outside `crates/`, the base reuses the change's binaries (`--build-base` builds it anyway), so the run checks that two runs of one build agree. It is `named_only` in `tools/parity_hosts.json`, since it fails any change that moves the picture on purpose: a pull request that must leave it alone carries the `frame-identity` label (docs/release-flow.md), and `python3 tools/parity.py frame_identity` runs it by name.

### Other machines

Harnesses can also run on the Mac mini and playhorizon (`tools/parity_hosts.json` describes them; `playhorizon` uses its CI runner's environment and vanilla data; its `default` list is empty, so pass `--hosts macmini,playhorizon` or set `PARITY_HOSTS`, after copying `shared-local/` to `~/code/horizon-parity/shared-local` on each host). The runner mirrors the worktree there with rsync (not `.git`, `.local`, `target` or ignored files) into `~/code/horizon-parity/remote/<host><path>`, whose `.local` links to the vanilla data in `~/code/horizon-parity/shared-local`, and starts `tools/parity.py --worker` over ssh. The worker builds the probes (about 80 s the first time) and runs what it is sent under the same tracer, admitting harnesses against its own free memory. One queue, longest first, feeds every machine, remote ones first: the Mac takes up to 14 harnesses, playhorizon 3 (on CPUs 0-3: its CI runner has 4-7). Each pass comes back with the content hash of every file it depended on, so it is keyed on exactly what ran, and logs come back to `.local/parity-runner/logs/`; state, sharing and `gate.py` work as before. If a host can't be reached, or drops out, its harnesses run here.

Harnesses a host can't run are listed in the file: `local_only` (`save_roundtrip` runs the game on a virtual screen) and each host's `exclude`. A host's `exclude` also holds when `parity.py` runs on that host itself, as its CI runner does: the harness is skipped there with its reason (playhorizon skips `remote_fireball` and `remote_fireworks`, which its CPU-starved CI slice fails even on bare main), unless named on the command line. The Mac mini's runner and the nightly audit still run them. The Mac excludes nothing: vanilla's `Math.sin`, `pow`, `log` and the like don't round the same on an Apple silicon Mac as on Linux (HotSpot uses fdlibm there, Intel's stubs on x86-64), and Rust's float functions round like neither (they call the platform libm), so code that mirrors a vanilla `Math` call uses `horizon_core::jmath`, which computes what HotSpot computes on the host architecture. The ThinkPad no longer runs harnesses (2026-10-26): playhorizon is the Linux host that is always on.

The file's `solo` list names harnesses that drive game windows (`remote_fireworks`, `server_player_sculk`, `world_reopen`, ...). On a machine where `parity.py` has 4 CPUs or fewer, such as playhorizon's CI runner, each of them runs alone, after the others: there frames are drawn on the CPU, and beside other harnesses a screenshot or a control request took more ticks than the check allows. `PARITY_SOLO=1` or `0` forces this on or off.

### Any command on playhorizon: `tools/remote.sh`

For Linux work that doesn't need a whole parity run (a crate's tests, a few harnesses, a server build), `tools/remote.sh <worktree> <command...>` runs the command on playhorizon at the worktree's commit and streams its output back; its exit status is the command's. Commit and push first: the host fetches the commit from GitHub, and nothing else leaves this machine (no files, no credentials, no environment). Each worktree keeps one checkout there, `~/remote-work/<name>` with its own `target/`, so the next run only rebuilds what changed, and one run per name goes at a time. The command runs niced in the host's CI slice, pinned to CPUs 0-3 (`--cpus`): the CI runner has 4-7, so these runs never slow a pull request's parity job. A benchmark asks for its CPUs with `--cpus 0-3 --exclusive` (and says so in its pull request): it starts only when no other run uses them, and later runs wait for it to end. The command gets scratch game and launcher homes and the vanilla data linked as CI links it; a command naming `parity`, `lan_e2e` or `horizon-prepare` also gets `.local/horizon-client`, prepared as CI prepares it. Whatever the command leaves running (a game, a daemon) is stopped when it ends, and stopping `remote.sh` (or losing the connection) stops the command on the host. Logs stay in `~/remote-work/<name>.logs/`; `--list` shows the checkouts, `--gc [DAYS]` removes old ones.

```sh
tools/remote.sh .claude/worktrees/my-fix cargo test -p horizon-world --lib   # 33 s, warm
tools/remote.sh . python3 tools/parity.py food                               # 19 s warm (4 min the first time); here: 188 s, 229 CPU-seconds
```

The host is lent by a partner who keeps hypervisor access: send it no secret, token or credential, and never take a result from it as a CI result.

### Shards and Rust scopes

A harness can declare, at the top level of its script:

- `PARITY_SHARDS = N`: it runs as `NAME#1` to `NAME#N`, in parallel, each told its share in `PARITY_SHARD=i/N` (`test_datafix_parity.py` upgrades every sixth world). `NAME` on the command line or in `--known-failures` means all of them.
- `PARITY_RUST_SCOPE = [...]`: the Rust sources its probes can reach, as path prefixes. Cargo's dep-info names every file of every crate a probe links, so without it a probe of `horizon-world` depends on all 350 of its files. The runner keeps only the scope's files if the scope is closed: nothing in it names a module, crate-root re-export or macro outside it (`crate::`, `super::`, other workspace crates; comments and strings ignored), declares a module it doesn't cover, or includes source, and nothing outside it implements anything for a type it defines. Crate roots then count without their declarations of other modules, so adding a module elsewhere doesn't rerun the harness. If the scope isn't closed the runner says why and keeps every file. `datafix` depends on 41 files this way, not 354.
- `PARITY_MANUAL = "<how to run it>"`: the script is a proof someone runs by hand with arguments (a Java side, then a native side against its reference), not a harness. `parity.py` leaves it out of every run, `--changed` and parity-main included, and names that text when it is asked for it by name.

### Which harnesses a change affects

`--changed` and `--changed-files` narrow each harness's Rust sources to those its probes can reach (`tools/parity_reach.py`), so a change to one module of `horizon-client` no longer reruns every harness whose probe links the crate. The narrowing works on items, not files: from the probe modules a harness ran (or the binary it built), a free item (fn, type, const, static) is reached when a reached item names it in its own module, through a path or an import of a module that is named, or through a glob import; an inherent impl's methods count one by one once their type is reached and their name is mentioned; a trait impl counts once its trait and its type are reached (std's traits and types always are); a macro counts once invoked. Names match by spelling, so the result only over-approximates. A reached file's parent modules count through their `mod` declarations of it, compared with the merge base; Rust files a change adds take part, a dependency that was deleted counts as changed, code under `#[cfg(test)]` never counts, and a harness whose probe can't be found keeps every source. Parses are cached by content in `reach-cache.json` of the shared parity folder (beside `passes/`), so a new worktree starts warm; a harness's search stops as soon as it reaches a changed file, and after `PARITY_REACH_BUDGET` seconds (default 180) the remaining harnesses count every source their probes link, so choosing never holds a slow machine for long. `--by-crate` turns the narrowing off. With `--changed`, `Cargo.lock` and `Cargo.toml` changes that only add, remove or move workspace crates (a crate split: no registry or git package, version, feature, dependency or build setting changed across them, compared with the merge base) don't count: the moved code counts through its sources. A workspace crate's feature that such a change defines or turns on is allowed only where every source naming it changed too (the new crate's own files). #2160's split went from 352 harnesses to 123 this way (its own changes to the runner still select them all). On the last 20 pull requests merged on 2026-10-25, the harnesses picked went from 2255 to 1061 (2384 to 1592 harness-minutes; game-binary harnesses, which link all of `game/`, set the floor), and every harness a CI parity run failed in the week before was still picked; every source file a sample of probes executed (LLVM `-C instrument-coverage`) is among those kept. Building the graph takes 15 s cold, under a second warm, and up to a minute and a half on a change that reaches many harnesses.

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

Debug builds carry line tables only (`[profile.dev]` in `Cargo.toml`), which keeps backtraces readable while keeping each worktree's build small; a fresh worktree builds `horizon-client` and its tests in under a minute.

## Server execution in game fixtures

`HORIZON_COMMAND_SCRIPT` runs on the integrated server's `Server thread`, with the
headless client joined through the normal memory connection. Commands use the connected
owner's body and dimension. The script owns the clock: each `tick` calls `tick_server`,
while login and portal arrival pump connection-only passes. The client acknowledges
terrain and teleports without predicting movement, matching the command oracle's stationary
player. Script and save errors return through server shutdown; output keeps the existing
styled messages and `CommandResultCallback` schema.

`HORIZON_DISPENSE_FIXTURE` and `HORIZON_SCULK_FIXTURE` are component probes. They construct
a server `Game` on a server thread and retain the oracles' synthetic levels, seeded random
streams and explicit entity/block phases. A synthetic sculk player controls warning levels,
not client input; dispenser probes call one activation without running unrelated world ticks.
Keep those controls when extending the fixtures. Player interactions belong in connected-client
coverage alongside the components:

```sh
python3 tools/test_command_exec_parity.py
python3 tools/test_dispenser_parity.py
python3 tools/test_sculk_parity.py
python3 tools/lan_e2e.py --check machines --check fireball --check sculk_container
```

The LAN checks open and fill machines through the guest's inventory packets, observe a
powered dispenser's projectile on both clients, and open a chest to activate a sculk sensor
and shrieker. They supplement the deterministic component comparisons.

A new `command_exec` group goes into a file of its own under `tools/command_exec_groups/`: a module
with `GROUP` (its name) and `cases(cmd, setup)` (its ops). The corpus finds it by file name, so two
pull requests that each add a group don't touch the same lines (#16833). Most existing groups are
still functions in `tools/command_exec_corpus.py`.

### Randoms that repeat

Vanilla seeds every unseeded random (`RandomSource.create()`: each entity's own, the level's, a raid's...) from `RandomSupport.generateUniqueSeed`, a shared uniquifier XORed with `System.nanoTime()`, and numbers entities from `ServerLevel.ENTITY_COUNTER`; Horizon ports both (`random_source::unique_seed`, `Game.next_entity`). A command script can make what they decide comparable exactly: a dropped item's position, primed TNT's kick, a dispensed item's motion, which tick of four a resting item moves on.

- `{"op": "sync_randoms", "uniquifier": U, "entity_id": E, "level_seed": S}`, run on the server thread of both engines, sets that thread's uniquifier to `U`, the entity-id counter to `E` (the next entity is `E + 1`) and the random of the player's level to seed `S`. Startup creates a different number of randoms in each engine, so sync right before what you compare, and give each case its own values (Horizon keeps an entity's UUID by id).
- A script with this step runs vanilla under `tools/seed_agent.py`'s agent (`SeedAgent.java`, built with the JDK's class-file API): `generateUniqueSeed` steps a uniquifier of the calling thread's own, from vanilla's start and by vanilla's factor, without the clock. Horizon runs with `HORIZON_TEST_SEEDS=1`, which does the same in `unique_seed`; the step fails without it. Per thread, so worker threads and Horizon's client don't shift the server thread's seeds. The agent jar is an oracle input (`tools/parity_shim.py`).
- `tools/seed_sync_corpus.py` is the example and the proof, a harness of its own (`python3 tools/parity.py seed_sync`): a script that syncs runs on fixed seeds to its end, so the groups after a sync would draw from them, and a group that passes on most seeds could fail on those every time. `--unsynced-randoms` leaves the steps out, and the same fields then differ run to run.

One draw too many or too few shifts every later value, so read the first field that differs. Not covered: randoms on other threads (worldgen and chunk loading: keep scenarios in loaded chunks), the levels other than the player's, randoms created before the step (the server's, the level's sound seeds) except the level's own, `Level.randValue` (random-tick positions) and other `ThreadLocalRandom` or `Math.random` draws, and anything a tick draws from the level's random before what you read (each tick shuffles the chunks near players with it, even with `spawn_mobs` off).

### State digest

A build with `--features state-digest` and `HORIZON_STATE_DIGEST=<file>` appends one line per level tick: hashes of the entities (join order), their own randoms, the block entities (ticker order) and the scheduled ticks, plus the level random, `Level.randValue`, the entity-id counter and the join counter. Off by default and compiled out. `digest-sync U E S` is `sync_randoms` (needs `HORIZON_TEST_SEEDS=1`). It works on a dedicated server's console and, through `horizon-ctl command`, on an integrated server (a LAN host), where it syncs the owner's level on the server thread, and `HORIZON_STATE_DIGEST_DETAIL=1` adds a raw line per entity and block entity. The join numbers are hashed relative to the sync, and the join field under `counters` is the upcoming join number. After the integrated `digest-sync` they are ranks instead: each entity's place in the join-order walk, and the number of entities visited. The window's client world shares the process-wide join counter there, so ranked digests match although the raw counters differ.

- `tools/state_digest_scene.py BINARY RESOURCES TEMPLATE OUT` runs a fixed dedicated-server scene (frozen before the first tick, `tick step N`) and writes `OUT/digest.txt`; `--generate` makes the forceloaded flat template once. Random ticks are off unless `--random-ticks`. `HORIZON_SCENE_GATE` names an optional script to wait on before each run.
- `--extended` adds subjects to that scene without changing the default one, so old baselines stay valid. Before the window: breeding pairs, primed TNT, skeletons beside an iron golem, zombies beside a villager, a zombie villager, a zombie, a piglin and a hoglin with conversion countdowns, creepers, a slime, a minecart, a boat with a rider, jockeys and a dispenser. Inside the window the script steps the frozen server in pieces and runs commands between them (`extended_plan` in the script): `/data merge` and `/data modify`, `/effect`, `/attribute`, `/item replace`, `/tp`, `/ride`, `/fill`, `/setblock`, `/damage`, `/kill`, `/execute ... run`, summoned arrows, snowballs, eggs and a fireball, lightning on a pig and a villager, and more TNT and creepers. Passes at ticks 0 to 100 need the default 200 ticks or more, and a pass at or after `--ticks` is skipped. It needs step mode, not `--sprint`. The script writes the console output to `OUT/commands.log` and counts lines that look like command errors in `timings.txt` (`command_errors`), so check that count is 0 before trusting a baseline. `/give` needs a player, so it stays in the players scene.
- `tools/state_digest_players.py ... --integrated` now syncs the host's randoms with `digest-sync`, so its digest can be compared run to run like the dedicated one. The run writes a digest even when the sync failed, so use it as a baseline only when `timings.txt` has `'digest_sync': 'ok'` (a failed sync records `'digest_sync': 'failed, ...'`). The integrated run also starts frozen (`HORIZON_STATE_DIGEST_FREEZE=1`) and makes the host a spectator.
- `tools/state_digest_compare.py A/digest.txt B/digest.txt` names the first differing step per group. Compare runs from one toolchain: the hashes are not stable across Rust versions.
- Keep the measured window free of chunk loading, generation and lighting, and of trial spawners, ominous item spawners and `/item modify` (clock-seeded UUIDs and loot random).

### Fuzzing scenarios

`tools/parity_fuzz.py` runs small generated scenarios in vanilla and Horizon and reports each divergence once, shrunk, with a reproduction (#12498). A batch of scenarios runs as one command-exec script (`test_command_exec_parity.py --groups-file`). It is a tool to run by hand, not a harness: `parity.py` never selects it.

```sh
python3 tools/parity_fuzz.py --smoke --binary target/fast/horizon-client.exe   # fixed scenarios that must match (~30 s)
python3 tools/parity_fuzz.py persist --seed 1 --count 60                          # summon or place, unload and reload, read
python3 tools/parity_fuzz.py spawn --seed 1 --count 60                            # each spawn mechanism, read tick by tick
python3 tools/parity_fuzz.py shulker --seed 1 --count 60                          # shulkers hit by a bullet, and shooting at a player
python3 tools/parity_fuzz.py items --seed 1 --count 60                            # one item stack with a component patch, by one route
python3 tools/parity_fuzz.py blockdata --seed 1 --count 60                        # a block entity's optional and nested fields
python3 tools/parity_fuzz.py retype --seed 1 --count 60                           # a saved field given back as another NBT tag
python3 tools/parity_fuzz.py commands --seed 1 --count 60                         # commands walked from the server's command tree
python3 tools/parity_fuzz.py --replay scenarios.json                               # run these scenarios (a report's, edited)
```

- **Scenarios.** A scenario is a seed and steps: summon (with NBT), place a block (with NBT), a command, ticks, a dispenser activation, an unload and reload, and reads (`/data get`). Each gets a chunk of its own far from the player (x 4096 and up, y 300, on a stone floor), forceloaded, and syncs both engines' randoms first ("Randoms that repeat"). A reload removes the forced ticket, ticks until both engines saved and dropped the chunk (a guard checks vanilla did), and loads it back: a real save and load through each engine's chunk and entity storage. Summoned entities get fixed UUIDs, so reads find them without selectors.
- **Families** (`tools/parity_fuzz_scenarios.py`). `persist` summons an entity or places a block entity with changed fields, maybe beside other blocks, and reads it before and after a reload. `spawn` makes an entity one way (summon, dispenser, dispenser on a redstone signal, a block broken, an explosion, a piston) and reads it before its first tick and after each of a few. Field names are vanilla's own: the first run summons every entity type and places every block entity bare and caches what vanilla saves for each in the work directory (`defaults.json`); mutations change those fields' values and keep their NBT types. `shulker` puts 2 to 6 shulkers close together, hits one with a live shulker bullet or uses a bullet as the source in `/damage ... by <bullet>` and counts shulkers after each tick, or teleports the player beside one shulker with its AI and reads its bullets and lid; a `{uN}` in a command and `{iuN}` in a summon's NBT stand for the UUID of the entity summoned as `n=N`. `items` (`tools/parity_fuzz_data.py`) builds one stack with a component patch (entries equal to the item's prototype, different from it, or removals), by `/item replace ... with`, `/item modify`, an inline loot table, a container's NBT, `/data merge` or an item entity; reads it, tests it with item predicates (`/execute if items`), and then copies it, lets a hopper move it, drops it, edits it or reloads it. The items, their prototypes and half of the values come from the prepared `items.json` (every item, and for each component every value some item's prototype gives it), so its seeds mean the same scenarios only for the same prepared resources; a report's scenarios replay anywhere. `blockdata` places one block entity with fields `persist` never sets (text, locks, loot tables, nested stacks, stored components, values out of range or of another type), by `/setblock` or `/data merge`. `retype` takes one or two of the fields vanilla saves for an entity or block entity and gives them back as another NBT tag: the same number in another type, the type's limits, a string for a number. `commands` (`tools/parity_fuzz_commands.py`) walks the prepared `commands.json` (vanilla's command tree) from a root to a node that runs and fills each argument by its parser: positions and entities are the scenario's own fixture (a barrel, a chest, a block, an armor stand, a pig, an item entity), names come from the prepared registries, stacks and predicates from `items`. Roots that change the world beyond the scenario's chunk or the player are left out, as are arguments that name players. Each command's feedback and results are compared, then every fixture is read. A run of 60 scenarios takes a few minutes; shrinking adds one or two batches per round.
- **Comparison.** A `/data get` read is parsed as SNBT and compared field by field, so a difference names its field. Key order is not compared, nor the order of `attributes` (vanilla's own order changes run to run). Other commands compare like the harness (feedback, styles, results). `ALLOWED` in the script lists the differences that are not bugs, each with its reason.
- **Grouping and shrinking.** Each diverging read is grouped by family, subject type, whether the read came after a reload, and first differing field. Each group's smallest scenario is shrunk (steps and NBT keys dropped, tick counts halved) while that group's divergence stays, trying all edits of a round in one batch. `report.json` in the work directory lists the groups, largest first, each with its seeds, its shrunk scenario as commands, and its differences.
- **Labels and refusals.** A family can label a step (`subject`), which replaces the subject type in the group's name: `items` names the route (`with`, `loot>reload`) and the kind of predicate. A step marked `setup` that vanilla refuses is counted ("vanilla refused the setup of N"), not hidden: a setup Horizon accepts and vanilla refuses is still a difference. `found.json` lists every group with its seeds and each seed's first differing command before shrinking starts.
- **On hosted runners.** `parity-hosted.yml` runs harnesses by name and keeps only the log of one that did not pass, so the fuzzer runs there as a probe: on a `parity-probe/*` branch (never main), add `tools/test_fuzz_probe_parity.py` with one slice per shard, and start the workflow with the shards' names (`gh workflow run parity-hosted.yml --ref parity-probe/<name> -f harnesses="fuzz_probe#1 fuzz_probe#2"`). Each shard prints the fuzzer's output, `found.json` and `report.json` into its log (the `parity-hosted-logs-<n>` artifact) and fails on purpose. A runner takes 8 shards at a time. Measured on 16 vCPU (2026-11-23): a batch of 25 scenarios takes about 100 seconds, so a shard that only searches (`--shrink-rounds 0`) fits about 200 scenarios in its 1,800 seconds, and one that also shrinks one round about 50; 16 shards of 50 with one shrink round took 33 to 63 runner-minutes. Search wide without shrinking, then shrink one seed per cluster in a second run. `--ordinary` keeps `items`, `blockdata` and `commands` to what a command or a played world gives (no wrong tag types, unknown ids or hand-written saved stacks), whose differences otherwise lead every round. `python3 tools/parity_fuzz_probe.py merge LOGS...` joins the shards' downloaded logs into clusters (family, step, field without ids and indices, how it differs).

  ```python
  PARITY_SHARDS = 2
  from parity_fuzz_probe import run
  run([['items', '--ordinary', '--seed', '1', '--count', '200', '--batch', '25', '--shrink-rounds', '0'],
       ['items', '--seed', '318', '--count', '1', '--shrink-rounds', '3']])
  ```
- **A workflow of its own.** `parity-fuzz.yml` (docs/ci-runners.md, "The fuzzer's workflow") needs no probe branch or stub: `gh workflow run parity-fuzz.yml --ref <branch> -f family=commands -f seed=1 -f count=200 -f runners=2` builds the game and the preparer once, in a job without the data, runs 8 slices of `count` seeds on each runner (`tools/parity_fuzz_probe.py slices`), and its summary job merges every slice's `found.json` into the clusters. `-f seeds=1017,1325 -f shrink_rounds=4` runs those seeds again, one slice each, shrunk (on the commit that found them: a change to a generator changes what a seed makes). A seed may be named once (`1` and `01` are one seed). A slice that found differences is a result; the run fails only when a slice compared nothing, or when the secrets for the data are missing. An entity's `Tags` are compared as a set (vanilla keeps them in a hash set).
- **Triage before filing.** A group is a lead, not a bug: read the vanilla source for the field, check it isn't a seed-sync limit (above) or harness noise, and search existing issues first.

## Reading vanilla

Game jars, decompiled sources and reports live in the gitignored `.local/`, rebuilt from your own copy of the game;
nothing of Mojang's is committed or downloaded by the tools ([README](native-client.md#run-from-a-checkout)):

```sh
python3 tools/lab.py prepare --server-jar ~/Downloads/server.jar   # checks the pinned jars (research-lock.json)
python3 tools/lab.py decompile    # Vineflower: .local/minecraft/26.3/sources/{client,server}/
python3 tools/lab.py status       # coverage reports under .local/minecraft/26.3/reports/
```

`tools/rea_vanilla.py` queries the shipped bytecode through [REA](https://github.com/morluto/rea) (a headless JADX)
for what text search misses: every caller of a method including lambdas and bridges, every overload, or a second
decompiler's view of a method whose Vineflower output looks wrong:

```sh
python3 tools/rea_vanilla.py class net.minecraft.world.level.block.DispenserBlock
python3 tools/rea_vanilla.py method net.minecraft.world.level.block.DispenserBlock dispenseFrom
python3 tools/rea_vanilla.py callers net.minecraft.server.level.ServerLevel tickChunk
python3 tools/rea_vanilla.py --jar server-game classes DedicatedServer
```

REA and JADX are pinned in `research-lock.json` and downloaded into `.local/tools/` on first use; each query takes
about five seconds. Decompiled bodies are a reading aid: the bytecode and the oracle harnesses are the reference.

## Player-facing integration and deterministic fixtures

`python3 tools/lan_e2e.py` runs every LAN check with an integrated server thread,
the same path as normal single player. `python3 tools/test_friends.py --client
target/fast/horizon-client --relay target/fast/horizon-relay --baseline` covers
plain LAN, direct invitations, a local relay and ended invitations in that mode.
Run Linux games under `xvfb-run -a` when no display is available. Each
process gets scratch game, launcher, XDG data and config homes; audio
and launcher self-update are disabled.

`lan_integration` and `friends_integration` register these suites with `parity.py`.
`friends_integration` (under a minute) is selected like any harness when the
drivers or game implementation change. `lan_integration` is the whole LAN suite
(~40 minutes on a loaded runner, and single checks flake under load), so
`tools/parity_hosts.json` lists it under `named_only`: no automatic selection
(`--changed`, `--failed`, `--flake-check`, CI, parity-main) picks it; it runs
when named and in the nightly audit (`--all`). LAN writes `results.json`
with the host mode and each result beside its screenshots and logs. A skipped
case is recorded explicitly and does not count as passing coverage.

`known_flaky` in `tools/parity_hosts.json` lists harnesses that fail now and then for a
cause not fixed yet, each with its reason. One that fails runs once more from
scratch, and a pass then counts. `parity.py` prints a `RETRIED` line for it
in its summary and in the CI job's summary (PR parity and parity-main), so the
flake stays visible. No other harness gets this retry. Fix the cause, then take
the harness off the list.

`known_failing` in `tools/parity_hosts.json` lists harnesses, or single shards
(`datafix#2`), that fail on main until an open issue is fixed, each as `"#issue:
what differs"`. They still run and report (a `KNOWN FAILING` line in the summary
and the CI job's summary), but their failure doesn't fail the run, so
parity-main, and the Nightly it gates, stop only for new failures, and
parity-main's bisect leaves them out. Without the list, one regression kept
every later parity-main red: each run rechecked every merge since the last green
one, and no Nightly could ship (2026-11-15 to 11-17). An entry covers any
failure of what it names, in pull requests' parity too, a new one included: list
the failing shard rather than the whole harness, and take the entry off with the
fix. A listed harness that passes gets a line saying so. Add a harness only with
its issue and only when the difference doesn't harm players (crashes, saves,
multiplayer): those block.

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
