# Release flow

How a change gets from a pull request to players: the checks a pull request passes, how it merges, and the
automatic releases that follow. Releases are automatic and frequent so the latest Horizon is always playable;
there is no separate development channel.

## Pull request checks

The `checks` workflow (`.github/workflows/ci.yml`) runs two jobs on every pull request:

| Job | Runs on | What | Time |
| --- | --- | --- | --- |
| `verify` | GitHub `ubuntu-latest` | Public-tree check, Python tests, rustfmt, clippy with warnings denied, Rust tests. Dependencies and the workspace build come from a cache saved by main's runs | ~4 min cold, less warm |
| `parity` | The Mac mini's runner (`ludovics-mac-mini`, label `horizon-game`) | The vanilla harnesses the change can affect (`tools/parity.py --changed <base>`) | seconds when nothing is affected; minutes per affected harness |

Pushes to main run `verify` only; the nightly audit covers main's harnesses.

### The parity job

Horizon never downloads the game, so harnesses that compare with vanilla run only where the maintainer's own
copy is: the Mac mini. Its runner runs as the maintainer's account and keeps its checkout between runs
(`clean: false`), so the probes build incrementally into `.local/parity-target` and earlier passes stand.

1. `tools/ci/runner-env.sh` puts the machine's Python 3.13 and the JDK `research-lock.json` pins on the PATH.
2. `tools/ci/vanilla-data.sh` links `.local/` to the runner's copy of the vanilla data, cloned once from
   `~/code/horizon-parity/shared-local` into `~/.actions-runners/horizon/_work/horizon/shared-local`, and
   prepares `.local/horizon-client` with the pull request's own `horizon-prepare` once per preparer version
   (its sources, the exporters in `mods/horizon/src/test/java` and `research-lock.json`). The game's sounds, which
   depend only on the Minecraft version, are kept once in `shared-local/sounds/<version>/` (copied from a folder
   `horizon-prepare sounds` made from the maintainer's own game; the Mac mini has no official launcher of its own)
   and cloned into each preparation (not linked: vanilla's packs refuse symbolic links), for the audio harnesses.
3. `tools/parity.py --changed <base sha>` picks the harnesses whose traced dependencies (their script, Python
   modules, Java oracles and every Rust source their probes compile) changed since the merge base, plus any
   harness nothing is recorded about yet, and runs those that have no pass for their current inputs. A change to
   `Cargo.lock` or `research-lock.json` affects every harness. Each harness has 25 minutes, the job 45.

Failed harnesses' logs are uploaded as the `parity-logs` artifact.

**Platform-specific harnesses.** A harness listed under `linux_only` in `tools/parity_hosts.json` gives
different results on macOS for a reason that is not a Horizon bug (the reason is printed). `parity.py` skips it on
other platforms unless it is named on the command line. Today it holds only `save_roundtrip` (it needs `vscreen`):
`options`, `chunk_generation`, `world_border_render` and `mob_ai`, which once differed on macOS through the
platform's libm, pass on the Mac mini since `horizon_core::jmath` computes what HotSpot computes (checked
2026-10-14).

**Pull requests from forks** never run code on the Mac mini by themselves. A maintainer who has read the change
adds the `run-parity` label: that runs the job once, on the commit the pull request had then. To run it again
after new pushes, remove the label and add it again.

**Two runners.** The Mac mini has two runners for this repository, `ludovics-mac-mini`
(`~/.actions-runners/horizon`, `CARGO_BUILD_JOBS=10`) and `ludovics-mac-mini-2` (`~/.actions-runners/horizon-2`,
`CARGO_BUILD_JOBS=6`), with the same labels, so the 30-minute nightly audit or a long parity job never holds a
release's macOS build back. Each keeps its own checkout, `target/` and Rust install; the second's
`_work/horizon/shared-local` links to the first's, so they share the vanilla data, the preparations (one at a
time, under a lock) and the parity passes and slots.

## Merging

Pull requests merge with a merge commit. The orchestrator sessions review a pull request, then turn on
auto-merge (`gh pr merge <n> --auto --merge`): GitHub merges it once the required checks pass. Branch
protection on main requires `verify`. Branches don't have to be up to date with main: two sessions merge in
parallel, and main's own `checks` run catches the rare semantic conflict.

`parity` becomes required once it has run green on pull requests for a while (a week without a false failure):

```sh
gh api -X POST repos/ludovic111/horizon/branches/main/protection/required_status_checks/contexts -f 'contexts[]=parity'
```

A skipped `parity` (a fork without the label, a run started by a label) counts as passed, so requiring it never
blocks a fork's pull request; the maintainer decides whether to run it.
