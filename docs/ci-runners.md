# CI runners

GitHub gives the organisation 20 hosted jobs at a time, shared by every pull request, every push to main and the
bots. On 2026-11-03 that pool had 283–351 jobs waiting (median wait 43–55 minutes) for jobs that work 1–18
minutes, and the bots' two small runners had 327–371 waiting. Since then (the maintainer's decision: "use everything
we have to the max") every job that can runs on our own machines, and GitHub's pool keeps only what can't move.

| Runner | Machine | Labels | Takes |
| --- | --- | --- | --- |
| `ludodesktop-checks-1`…`-3` | ludodesktop (the maintainer's desktop, 24 threads), isolated user `gha` | `horizon-checks` | `checks`' `verify` and `launcher` for the pull requests whose number ends in 0 or 1, forks included, `mod-registry`, `pack load bench` |
| `playhorizon-light-1`…`-12` | playhorizon (partner's server, 8 vCPU) | `horizon-light`, `playhorizon-light` | The bots that run main's own scripts: `pr labels`, `overlap note`, `cancel-on-close`, `claim command`, `triage command`, `mojang republish`, `nightly build`'s `answer`, and `checks`' `parity-scope` |
| `playhorizon-ci-1`, `playhorizon-ci-2` | playhorizon | `playhorizon-ci` | Nothing routed since 2026-11-03 (they took `checks` of this repository's own pull requests with the old `SELF_HOSTED_CI=all`) |
| `playhorizon-horizon`, `ludodesktop-horizon`, `ludodesktop-parity-2` | playhorizon, ludodesktop | `horizon-game` | Parity ([release-flow.md](release-flow.md#the-parity-job)) |
| `ludovics-mac-mini*` | Mac mini | `horizon-game`, `horizon-parity-main`, `horizon-release`, `horizon-nightly` | Parity, parity-main, macOS builds, releases |

The desktop's runners are declared in the maintainer's NixOS configuration (his private fleet repository,
`nixos/ludodesktop/horizon-runner.nix`), not installed from here; playhorizon's by `tools/ci/checks-runners.sh`.
Every persistent checkout (the desktop's `/var/lib/gha/<runner>/horizon`, playhorizon's, the Mac mini's) is kept small by
[`tools/ci/git-slim.sh`](../tools/ci/git-slim.sh) before a job or while the runner is idle; the desktop runs its own copy
from a `ACTIONS_RUNNER_HOOK_JOB_STARTED` hook in the NixOS configuration.

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
- **Main's pushes and eight of ten pull requests' `verify` and `launcher`** (number ending in 2–9): `checks` runs
  ~100 times an hour (2026-11-03: 21 pushes to main and 76 pull request runs an hour), each with `verify` (10–18
  minutes of work on GitHub, 11–13 on a desktop runner with a cold build folder) and `launcher` (1–11). Three
  runners give ~180 runner-minutes an hour: about the fifth of the pull requests they take. Main's pushes also save
  the Rust cache GitHub's runs of pull requests read. `SELF_HOSTED_CI=all` sends them all to ours.

## Which jobs, and when

The repository variable `SELF_HOSTED_CI` decides, from the next job queued (queued jobs keep the runner they asked
for):

| `SELF_HOSTED_CI` | Bots (`horizon-light`) | `checks`, `mod-registry`, `pack load bench` (`horizon-checks`) |
| --- | --- | --- |
| unset (default) | ours | `verify`, `launcher`: ours for this repository's own pull requests ending in 0 or 1, GitHub's for the others, forks' and main's pushes; the rest ours, forks' pull requests excepted |
| `all` | ours | ours, every pull request |
| `light` | ours | GitHub's: the desktop is paused |
| `off` | GitHub's | GitHub's: everything back, in one switch |

```sh
gh variable set SELF_HOSTED_CI --body light --repo horizon-rust/horizon   # or all, off
gh variable delete SELF_HOSTED_CI --repo horizon-rust/horizon             # the default
```

Set `light` when the desktop is paused (the maintainer uses it), `off` when playhorizon is down or moves: the bots
would wait for it. Back to the default when they return.

The Python step uses `unittest --durations` (Python 3.12+); the self-hosted `all` route must provide Python 3.12 or newer.

## Forks

A fork's pull request runs on GitHub's runners (since 2026-11-10). GitHub gives it no repository variable, so
`SELF_HOSTED_CI` can't move it: when the desktop was off or paused, the required `verify` of every fork's pull
request ending in 0 or 1 waited for it until GitHub failed the job a day later, and re-running it waited again
(#14131 sat on `horizon-checks` with `SELF_HOSTED_CI=light`). The jobs check
`github.event.pull_request.head.repo.full_name == github.repository` before they pick our runners.

A fork's pull request is code nobody here wrote. Its own copy of the workflows could still name our labels, so the
checks runners are the only ones it may reach:

1. The checks runners are an isolated system user on the desktop (`gha`: no sudo, no logins, no SSH keys, `/home`
   invisible, the rest of the system read-only, no GPU or device), and their jobs' tokens write nothing and see no
   secret (`tests/test_ci_runners.py`).
2. The light runners take main's own scripts only: the bots run on `pull_request_target`, `issue_comment`,
   `schedule` and main's pushes, which always run main's workflow and main's scripts, never the pull request's
   (each workflow's header says how); `checks`' `parity-scope` only for this repository's own branches. The test
   pins both.
3. A fork's pull request runs its *own* copy of the workflows, so it could name the light labels anyway. Before
   every job, playhorizon's runners' `job-started.sh` (`ACTIONS_RUNNER_HOOK_JOB_STARTED`) fails any pull request
   event whose head repository isn't this one, before a line of it is on disk.
4. The repository asks a maintainer to approve the runs of every outside contributor's pull request (Settings →
   Actions → "Require approval for all external contributors").

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
- `job-started.sh`, before every job: refuses a fork's pull request; empties a cargo build folder over 40 GB
  (`MAX_TARGET_GB`); collects a checkout's `.git` over 1.5 GB with `git-slim.sh` (`git gc --prune=now`, installed beside
  the hook; it also folds many packs and removes the `tmp_pack_*` of cut-short fetches) and drops one still over 4 GB
  (`MAX_GIT_GB`), which the checkout then clones again; removes the runner's diagnostic logs older than a day.
  actions/checkout turns gc off in a persistent checkout and every job's fetch adds a pack: on 2026-11-03 each light
  runner's checkout held 6822 packs, 50 GB; on 2026-11-09 the desktop's checks runners held 1051 packs (111 GB in one).
- The checks runners share one cargo home (`~/.actions-runners/ci-shared/cargo`: registry and git sources). Each has
  its own rustup home and its own build folder (`CARGO_TARGET_DIR`, outside the checkout). `CARGO_BUILD_JOBS=4`.
- Python is the host's 3.13 (`ci-shared/bin/python3`); the apt packages the jobs need are installed by the script, so
  the jobs skip their `apt-get`, `setup-python` and Rust cache steps there (`runner.environment == 'github-hosted'`).

## On the desktop

`gha.slice` holds every runner there (parity and checks): CPU and I/O weight 20, 22 of the 24 threads (12 while the
CPU is at 92 °C or more), memory 24 GB soft / 26 GB hard. Each checks runner has `CARGO_BUILD_JOBS=6`, its own
build folder (`/var/lib/gha/checks-<n>/target`, kept between jobs, emptied past 30 GB before the runner starts), its
own rustup and cargo homes, Python 3.13, gh (no login), the libraries the game and GPUI link against, and its own
Xvfb with Mesa's software Vulkan. They run all the time; the maintainer pauses them when he uses the desktop:

```sh
ssh desktop 'sudo -n touch /var/lib/gha-control/pause && sudo -n systemctl stop gha-runners.target'   # pause
ssh desktop 'sudo -n rm -f /var/lib/gha-control/pause && sudo -n systemctl start gha-runners.target' # resume
```

With the desktop paused, set `SELF_HOSTED_CI=light`, or `checks` waits for it.

## When our runners are offline

`.github/workflows/runner-watch.yml` runs every 30 minutes on GitHub's runners (never ours) and calls
`tools/ci/runner_watch.py`. When a job has waited over 2 hours for a self-hosted runner, in a pool where no job is
running at all (offline, not just busy), it opens one issue, "CI: self-hosted runners are not taking jobs", listing
each pool, what stops and what to do, and keeps its text current; it closes the issue once nothing waits. It needs
only the workflow token (`actions: read`, `issues: write`). What the Team and the council do meanwhile:
[operations.md](operations.md#self-hosted-runners-are-offline).

## Rerunning a failed job

GitHub reruns a failed job only after its whole run has ended: while another job of the run (often `lint`) is still
going, "Re-run failed jobs" is refused ("already running"). Wait for the run to finish, then rerun the failed job.

## Rust version

CI, the launcher's native-mod toolchain and the wasm mods' toolchain use one Rust release, written once in
`tools/ci/rust-version`: the prebuilt game must be built by the compiler that builds native mods against it, and a
floating `stable` changed the lints and the build layout under running jobs. `tools/ci/rustup_install.sh` installs it
and makes it each job's toolchain; `launcher/src/rust_toolchain.rs` and `crates/cargo-horizon-mod/src/wasm.rs` read it
at compile time; `tests/test_rust_pin.py` fails on a workflow that names a channel or a release of its own.

To move to a new release, in one pull request:
1. write the new number in `tools/ci/rust-version`;
2. update `DIST` and each platform's component checksums in `launcher/src/rust_toolchain.rs` for that release;
3. fix what the new compiler and Clippy report, and anything the release changed (build layout, warnings).

## Cheap checks on contributors' pull requests

A pull request whose head branch is in a contributor's fork gets only the cheap checks: `verify-static` and
`format` (repository hygiene, the mod API boundary scan and rustfmt; nothing compiles). `lint`, `verify`,
`launcher`, the Windows launcher tests, the modding book and cargo-horizon-mod's builds are skipped, and a skipped job
counts as passed for a required check. Everything runs on main's pushes, on dispatches, and on pull requests from
this repository's own branches (rollups, syncs, fast-path fixes, our sessions' work), which carry contributors'
changes into one heavy run. The rule is `tools/ci/pr_gate.py`; `tests/test_pr_gate.py` keeps the workflows to it.

A reviewer runs the heavy gate on one contributor's pull request with `gh workflow run dyna-ci.yml -f pr=<number>`.
Main's workflow checks out that pull request's head commit on GitHub's runners (never ours) with a read-only token,
and the `pr-status` job, which runs none of its code and alone may write commit statuses, posts `dispatched/lint`,
`dispatched/verify`, `dispatched/verify-static` and `dispatched/launcher` on that commit.
