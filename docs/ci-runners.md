# CI runners

GitHub gives the organisation 20 hosted jobs at a time, shared by every pull request, every push to main and the
bots. On 2026-11-03 that pool had 283–351 jobs waiting (median wait 43–55 minutes) for jobs that work 1–18
minutes, and the bots' two small runners had 327–371 waiting. Since then (the maintainer's decision: "use everything
we have to the max") every job that can runs on our own machines, and GitHub's pool keeps only what can't move.

| Runner | Machine | Labels | Takes |
| --- | --- | --- | --- |
| `vps-checks-1`, `vps-checks-2` | the CI VPS (14 vCPU, 62 GB, no GPU), isolated user `gha` | `horizon-checks` | `mod-registry`, `telemetry-server`, `pack load bench`; `checks`' `verify` only with `SELF_HOSTED_CI=all` |
| `vps-light-1`…`-8` | the CI VPS | `horizon-light` | The bots, as playhorizon's light runners below |
| `playhorizon-light-1`…`-12` | playhorizon (partner's server, 8 vCPU) | `horizon-light`, `playhorizon-light` | The bots that run main's own scripts: `pr labels`, `overlap note`, `cancel-on-close`, `claim command`, `triage command`, `mojang republish`, `nightly build`'s `answer`, and `checks`' `parity-scope` |
| `playhorizon-ci-1`, `playhorizon-ci-2` | playhorizon | `playhorizon-ci` | Nothing routed since 2026-11-03 (they took `checks` of this repository's own pull requests with the old `SELF_HOSTED_CI=all`) |
| `playhorizon-horizon` | playhorizon | `horizon-game` | Parity ([release-flow.md](release-flow.md#the-parity-job)) |
| `ludovics-mac-mini*` | Mac mini | `horizon-game`, `horizon-parity-main`, `horizon-release`, `horizon-nightly` | Parity, parity-main, macOS builds, releases |

ludodesktop (the maintainer's desktop) runs no Horizon CI since 2026-11-17 (his decision): its runners were removed.
playhorizon's runners are installed by `tools/ci/checks-runners.sh`. Every persistent checkout (playhorizon's, the Mac
mini's) is kept small by [`tools/ci/git-slim.sh`](../tools/ci/git-slim.sh) before a job or while the runner is idle.

Blacksmith's runners (outside GitHub's 20-job pool) took over from GitHub's on 2026-11-17 for jobs that hold no
secret and neither publish nor release: `checks`' `verify` ([below](#verify-on-blacksmith)), `perf-wasm`,
`cargo-horizon-mod`'s `plan`, `needs-info`, and the bots whenever `SELF_HOSTED_CI=off` sends them away from ours.

## What stays on GitHub, and why

- **Every job that holds a secret, or whose token can push, tag, publish or start workflows** (`contents: write`,
  `actions: write`): `release`'s and `launcher`'s `decide` (they choose the commit a release builds and start the
  build), `nightly build`'s `pick`, `sign` and `publish`, `launcher`'s `sign` and `publish`, `nightly command` (it
  writes the set of pull requests the Nightly holds), `native hook stamp` (it pushes a branch). playhorizon is a
  partner's server that the partner can reach, and a checks runner keeps what a fork's job left behind: neither may
  decide or sign what players download. `tests/test_ci_runners.py` pins it.
- **Release builds**: Linux on `ubuntu-22.04` (players' binaries must link against its older glibc; our Linux
  machines have newer ones), Windows on `windows-latest` (no Windows machine). macOS builds already run on the Mac
  mini.

## verify on Blacksmith

`checks`' `verify` runs on Blacksmith's runners (blacksmith.sh sponsors our minutes) for every pull request, forks'
included, the whole merge queue and main's pushes: ~100 runs an hour on 2026-11-03, more than the CI VPS's two checks
runners could take. They are GitHub's image on faster machines, outside GitHub's 20-job pool, so `verify` is one job
per part, side by side, each on a size that fits it:

| Job | Runner | Merge queue |
| --- | --- | --- |
| `verify (tree and Python tests)` (`verify-tree`) | 8 vCPU: the Python tests run one module per process (`tools/ci/unittest_parallel.py`) | yes |
| `verify (format and lint)` (`verify-lint`) | 16 vCPU | no |
| `verify (server build lint)` (`verify-server`) | 16 vCPU | no |
| `verify (launcher)` (`verify-launcher`), when `tools/ci/rust_changes.py --launcher` says the change reaches it | 16 vCPU | no |
| `verify (Rust tests)` (`verify-test`) | 16 vCPU | yes |
| `verify (mod points)` (`verify-mod-points`): the comparison runs on pull requests only; elsewhere it builds the generator to warm its disk | 16 vCPU | yes |

Only `verify`, which waits for them all, is a required check; `tests/test_ci_runners.py` pins that the parts share one
`runs-on` choice. `SELF_HOSTED_CI=all` sends them all to our checks runners.

Each Rust job keeps its Cargo build folder between runs on a Blacksmith sticky disk, as the VPS keeps its own
(`tools/ci/build_folder.sh`): one disk per job, so jobs that ran side by side don't replace each other's build. A
checkout dates every file now; the script gives each file whose Git object is the one the disk's last job saw that
job's date back (`tools/ci/source_mtimes.py`), so Cargo rebuilds only what changed, and empties a disk past 50 GB
(`MAX_TARGET_GB`). A failed or cancelled job leaves its disk as it found it. Only main's pushes and the merge queue
ask to save the disks (`SAVE_BUILD_DISK` in `ci.yml`); pull requests and [try runs](#try-runs) say `commit: false`,
build on main's latest and discard what they add. The organisation's branch-protected caches setting also refuses,
on Blacksmith's side, a pull request's save ("... not a trusted trigger" in its logs) and that of a run on any other
branch than main ("... is not the default branch"), the merge queue's excepted: its save is accepted (run
44622983852). So a pull request's disks are warm only once main has pushed since the job's disk was created, and a
fork's copy of the workflow cannot write this repository's disks. Forks' pull requests mount main's disks as well:
until 2026-11-17 they had a set of their own that nothing could save, so each built the whole workspace (7-10 minutes
per Rust part, against ~2 for a branch here).

## Try runs

The way to compile and test a branch of this repository without a pull request, and without building on your own PC:
push the branch, then start `checks` on it. Only who can push here (the organisation's owners, and their agents) can
start one; a fork can't. Prefer it to a local build.

```sh
git push origin HEAD:try/<name>                               # any branch here works; try/ starts nothing by itself
gh workflow run checks --ref try/<name>                       # Linux: verify's parts
gh workflow run checks --ref try/<name> -f windows=true       # and the game for Windows x64, the launcher's tests there
gh run list --workflow checks --branch try/<name> --limit 1   # its id; then gh run watch <id>
gh run download <id> -n horizon-windows-x64                   # the Windows game, for 3 days
git push origin --delete try/<name>                           # when done
```

- **Linux**: verify's parts on Blacksmith, as for a push to main: the whole workspace's tests, clippy, the server build,
  the launcher, the Python tests and the whole history's tree check. Not the mod points comparison (pull requests only).
  They mount main's disks and never save them, so a branch near main takes about as long as a pull request: 7 minutes
  on 2026-11-19, the tree check of the whole history the slowest part. The Windows job took 48: 28 for the game, 4 for
  the folder, 15 for the launcher's tests.
- **Windows x64** (`-f windows=true`, `windows-x64`, on `windows-latest` as the release build): the game built as
  `game-build.yml` builds it, uploaded as the artifact `horizon-windows-x64` (kept 3 days): `horizon-client.exe`,
  `horizon-ctl.exe`, `horizon-prepare.exe`, `horizon-server.exe`, `horizon-wasm-std.wasm` and DXC. No exporters (they
  are compiled from Minecraft's jars on the Mac mini), so point it at resources an installed Horizon already prepared
  (`--resources`). Then the launcher's tests. It builds from scratch, with no Rust cache read or saved. The repository
  is public: anyone signed in to GitHub can download the artifact for its 3 days, so a try run is no private channel.
- `-f arm64=true` adds the Windows ARM64 job (about 90 minutes) and `-f jfr=true` the JFR analysis on the Linux game
  runner; every hand-started run ran both before the inputs existed.
- No job of a try run holds a secret; verify's parts and the Windows x64 job save no disk or cache
  (`tests/test_ci_runners.py`). Runs of one commit share a concurrency group: a second waits for the first.
- A try run reports `verify` on the branch's commit, as a pull request's run of that commit does. It is no proof for
  a pull request: it skips the mod points comparison, and the merge queue still needs its own green `verify` on the
  queue's commit, which a try run cannot give.
- The run uses the branch's own copy of `ci.yml`: a branch from before 2026-11-19 has no inputs and always starts the
  ARM64 and JFR jobs. Rebase it on main first.

## Which jobs, and when

The repository variable `SELF_HOSTED_CI` decides, from the next job queued (queued jobs keep the runner they asked
for):

| `SELF_HOSTED_CI` | Bots (`horizon-light`) | `checks`, `mod-registry`, `telemetry-server`, `pack load bench` (`horizon-checks`) |
| --- | --- | --- |
| unset (default) | ours | `verify`: Blacksmith's; the rest ours, forks' pull requests excepted |
| `all` | ours | ours, every pull request, `verify` included |
| `light` | ours | Blacksmith's |
| `off` | Blacksmith's | Blacksmith's |

```sh
gh variable set SELF_HOSTED_CI --body light --repo horizon-rust/horizon   # or all, off
gh variable delete SELF_HOSTED_CI --repo horizon-rust/horizon             # the default
```

It is `light` since 2026-11-17, when the desktop's runners were removed. Set `light` when the CI VPS is down, `off`
when playhorizon is down or moves: the bots would wait for it. Back to the default when they return.

## Forks

An approved fork's pull request runs `verify` on Blacksmith's runners, like this repository's own, on main's sticky
disks, which it cannot write ([above](#verify-on-blacksmith)). It ran on the CI VPS for numbers ending in 0 until
2026-11-17, and waited there 30 to 150 minutes when the VPS's two checks runners ran back to back. `mod-registry` and
`telemetry-server` check the head repository and run a fork's on Blacksmith's.

`SELF_HOSTED_CI` cannot move a fork's run (it sees no repository variable): with `all`, its `verify` still runs on
Blacksmith's. When our checks runners are off or paused, jobs waiting for them wait until they are back. `runner-watch.yml` opens an issue when no checks runner takes jobs
([below](#when-our-runners-are-offline)).

A fork's pull request is code nobody here wrote. Its own copy of the workflows could name any of our labels, so only the
checks runners may take it:

1. The checks runners are an isolated system user on the CI VPS (`gha`: no sudo, no logins, no SSH keys, `/home`
   invisible, the rest of the system read-only, no GPU or device), and their jobs' tokens write nothing and see no
   secret (`tests/test_ci_runners.py`).
2. The light runners take main's own scripts only: the bots run on `pull_request_target`, `issue_comment`,
   `schedule` and main's pushes, which always run main's workflow and main's scripts, never the pull request's
   (each workflow's header says how); `checks`' `parity-scope` only for this repository's own branches. The test
   pins both.
3. A fork's pull request runs its *own* copy of the workflows, so it could name the light labels anyway. Before
   every job, the runners' `job-started.sh` (`ACTIONS_RUNNER_HOOK_JOB_STARTED`) fails any pull request event whose
   head repository isn't this one, before a line of it is on disk, unless the runner's `.env` sets
   `HORIZON_RUNNER_POOL=checks` (only the checks runners do, the light ones never).
4. The repository asks a maintainer to approve the runs of every outside contributor's pull request (Settings →
   Actions → "Require approval for all external contributors").

## On the CI VPS

A VPS lent for CI since 2026-11-15 (Ubuntu 26.04, 14 vCPU, 62 GB, 400 GB, no GPU). Every runner is a system service
(`gha-runner-checks-<n>`, `gha-runner-light-<n>`) run by the isolated user `gha` (no sudo, no logins, `/home` hidden,
the rest of the system read-only), inside `gha.slice` (memory ≤ 56 GB). The before-job hook is `job-started.sh` from
`tools/ci/checks-runners.sh` unchanged. The two checks runners have `CARGO_BUILD_JOBS=7`, their own rustup homes and
build folders (`/var/lib/gha/checks-<n>/target`, emptied past 40 GB), one shared cargo home and Python 3.13. The eight
light runners have no Rust. No parity runner: without a GPU the game would render on the CPU.

## On playhorizon

Installed by `tools/ci/checks-runners.sh` (run it there; it reads the registration token on stdin, so the token never
touches the disk or the screen):

```sh
gh api -X POST repos/horizon-rust/horizon/actions/runners/registration-token --jq .token |
  ssh ludovic@playhorizon 'bash ~/horizon-install/checks-runners.sh install'
ssh ludovic@playhorizon 'bash ~/horizon-install/checks-runners.sh status'
```

- Each runner is a systemd user service (`horizon-runner-ci-<n>`, `horizon-runner-light-<n>`) in
  `~/.actions-runners/horizon-{ci,light}-<n>`, with its own work folder and `HOME`. `LIGHT_RUNNERS` (default 12) and
  `CI_RUNNERS` (default 2) say how many.
- All of them run nice and pinned to CPUs 4–7, inside `horizon-ci.slice` (the CPU quota and memory cap of all CI on
  the machine, `linux-runner.sh`). The checks runners have their own `horizon-ci-checks.slice` (memory ≤ 16 GB).
  CPUs 0–3 stay for `tools/remote.sh` runs, benchmarks and the servers.
- `job-started.sh`, before every job: refuses a fork's pull request unless `HORIZON_RUNNER_POOL=checks`; empties a cargo build folder over 40 GB
  (`MAX_TARGET_GB`); collects a checkout's `.git` over 1.5 GB with `git-slim.sh` (`git gc --prune=now`, installed beside
  the hook; it also folds many packs and removes the `tmp_pack_*` of cut-short fetches) and drops one still over 4 GB
  (`MAX_GIT_GB`), which the checkout then clones again; removes the runner's diagnostic logs older than a day.
  actions/checkout turns gc off in a persistent checkout and every job's fetch adds a pack: on 2026-11-03 each light
  runner's checkout held 6822 packs, 50 GB; on 2026-11-09 the desktop's checks runners held 1051 packs (111 GB in one).
- The checks runners share one cargo home (`~/.actions-runners/ci-shared/cargo`: registry and git sources). Each has
  its own rustup home and its own build folder (`CARGO_TARGET_DIR`, outside the checkout). `CARGO_BUILD_JOBS=4`.
- Python is the host's 3.13 (`ci-shared/bin/python3`); the apt packages the jobs need are installed by the script, so
  the jobs skip their `apt-get`, `setup-python` and build cache steps there (those run on Blacksmith's runners only,
  `startsWith(runner.name, 'blacksmith-')`).

## When our runners are offline

`.github/workflows/runner-watch.yml` runs every 30 minutes on GitHub's runners (never ours) and calls
`tools/ci/runner_watch.py`. When a job has waited over 2 hours for a self-hosted runner, in a pool where no job is
running at all (offline, not just busy), it opens one issue, "CI: self-hosted runners are not taking jobs", listing
each pool, what stops and what to do, and keeps its text current; it closes the issue once nothing waits. It needs
only the workflow token (`actions: read`, `issues: write`). What the Team and the council do meanwhile:
[operations.md](operations.md#self-hosted-runners-are-offline).
