# CI runners

GitHub gives the organisation 20 hosted jobs at a time, shared by every pull request, every push to main and the
bots. On 2026-11-03 that pool had 283–351 jobs waiting (median wait 43–55 minutes) for jobs that work 1–18
minutes, and the bots' two small runners had 327–371 waiting. Since then (the maintainer's decision: "use everything
we have to the max") every job that can runs on our own machines, and GitHub's pool keeps only what can't move.

| Runner | Machine | Labels | Takes |
| --- | --- | --- | --- |
| `vps-checks-1`, `vps-checks-2` | the CI VPS (14 vCPU, 62 GB, no GPU), isolated user `gha` | `horizon-checks` | `checks`' `verify` for pull requests whose number ends in 0 or 1 (this repository's own and approved forks'), `verify` of the whole merge queue, `mod-registry`, `telemetry-server`, `pack load bench` |
| `vps-light-1`…`-8` | the CI VPS | `horizon-light` | The bots, as playhorizon's light runners below |
| `playhorizon-light-1`…`-12` | playhorizon (partner's server, 8 vCPU) | `horizon-light`, `playhorizon-light` | The bots that run main's own scripts: `pr labels`, `overlap note`, `cancel-on-close`, `claim command`, `triage command`, `mojang republish`, `nightly build`'s `answer`, and `checks`' `parity-scope` |
| `playhorizon-ci-1`, `playhorizon-ci-2` | playhorizon | `playhorizon-ci` | Nothing routed since 2026-11-03 (they took `checks` of this repository's own pull requests with the old `SELF_HOSTED_CI=all`) |
| `playhorizon-horizon` | playhorizon | `horizon-game` | Parity ([release-flow.md](release-flow.md#the-parity-job)) |
| `ludovics-mac-mini*` | Mac mini | `horizon-game`, `horizon-parity-main`, `horizon-release`, `horizon-nightly` | Parity, parity-main, macOS builds, releases |

ludodesktop (the maintainer's desktop) runs no Horizon CI since 2026-11-17 (his decision): its runners were removed.
playhorizon's runners are installed by `tools/ci/checks-runners.sh`. Every persistent checkout (playhorizon's, the Mac
mini's) is kept small by [`tools/ci/git-slim.sh`](../tools/ci/git-slim.sh) before a job or while the runner is idle.

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
- **Main's pushes, and eight of ten pull requests' `verify`** (number ending in 2–9): `checks` runs
  ~100 times an hour (2026-11-03: 21 pushes to main and 76 pull request runs an hour), each with `verify` (10–18
  minutes of work on GitHub; the launcher's checks are one of its steps). The CI VPS's two checks runners give ~120
  runner-minutes an hour. Main's pushes also save the Rust cache GitHub's runs of pull requests read. `SELF_HOSTED_CI=all` sends them all to ours.

## The merge queue

The whole merge queue runs on the checks runners (about 11 queue runs an hour). On the VPS the build folder persists,
so a queue run's Rust tests take about 3 min against about 9 on GitHub's runners. If the VPS is down, set
`SELF_HOSTED_CI=light` and the queue goes back to GitHub's runners.

## Two jobs per pull request

`verify` is two jobs, not four, because GitHub's 20-job pool is what a burst of pull requests waits for and every job
spends about a minute on setup before it works. Measured 2026-11-13..15 (medians): Rust tests 8.8 min, format and
lint 4.0, public tree and Python tests 2.0, launcher checks under 1.5 min in 92% of runs (median 0.5). `verify (tree,
lint and launcher)` (`verify-static`) runs the three short parts as steps, each with its own condition (the merge queue
runs only the tree steps; the launcher steps only when `tools/ci/rust_changes.py --launcher` says the change reaches
it); `verify (Rust tests)` (`verify-test`) stays alone. Both use the same `runs-on` expression, which
`tests/test_ci_runners.py` pins. Only `verify`, which waits for both, is a required check.

## Which jobs, and when

The repository variable `SELF_HOSTED_CI` decides, from the next job queued (queued jobs keep the runner they asked
for):

| `SELF_HOSTED_CI` | Bots (`horizon-light`) | `checks`, `mod-registry`, `telemetry-server`, `pack load bench` (`horizon-checks`) |
| --- | --- | --- |
| unset (default) | ours | `verify`: ours for pull requests ending in 0 or 1 (forks' included, see Forks) and for the whole merge queue, GitHub's for the other pull requests and main's pushes; the rest ours, forks' pull requests excepted |
| `all` | ours | ours, every pull request |
| `light` | ours | GitHub's |
| `off` | GitHub's | GitHub's: everything back, in one switch |

```sh
gh variable set SELF_HOSTED_CI --body light --repo horizon-rust/horizon   # or all, off
gh variable delete SELF_HOSTED_CI --repo horizon-rust/horizon             # the default
```

It is `light` since 2026-11-17, when the desktop's runners were removed. Set `light` when the CI VPS is down, `off`
when playhorizon is down or moves: the bots would wait for it. Back to the default when they return.

## Forks

An approved fork's pull request whose number ends in 0 or 1 runs `verify` on our checks runners (the CI
VPS), like this repository's own; the others run on GitHub's. Why: GitHub's pool of 20
jobs was the only place fork pull requests waited (2026-11-15: 544 runs queued, fork pull requests waiting 7+ hours
while the VPS's two checks runners sat idle). The jobs no longer check the head repository before they pick our runners.

The risk accepted: fork code runs on an isolated runner user after a maintainer approved it, and the runner's build
folder persists between jobs. That is why nothing these runners build is shipped or signed.

`SELF_HOSTED_CI` cannot move a fork's run (it sees no repository variable): when our checks runners are off or paused,
those runs wait until they are back. `runner-watch.yml` opens an issue when no checks runner takes jobs
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
  the jobs skip their `apt-get`, `setup-python` and Rust cache steps there (`runner.environment == 'github-hosted'`).

## When our runners are offline

`.github/workflows/runner-watch.yml` runs every 30 minutes on GitHub's runners (never ours) and calls
`tools/ci/runner_watch.py`. When a job has waited over 2 hours for a self-hosted runner, in a pool where no job is
running at all (offline, not just busy), it opens one issue, "CI: self-hosted runners are not taking jobs", listing
each pool, what stops and what to do, and keeps its text current; it closes the issue once nothing waits. It needs
only the workflow token (`actions: read`, `issues: write`). What the Team and the council do meanwhile:
[operations.md](operations.md#self-hosted-runners-are-offline).
