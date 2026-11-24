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
`cargo-horizon-mod`'s `plan`, `needs-info`, `stale pull requests`, and the bots whenever `SELF_HOSTED_CI=off` sends them away from ours.
One exception holds secrets there: [hosted parity](#hosted-parity), started by hand, whose fetch step reads the
maintainers' vanilla data.

## What stays on GitHub, and why

- **Every job that holds a secret, or whose token can push, tag, publish or start workflows** (`contents: write`,
  `actions: write`): `release`'s and `launcher`'s `decide` (they choose the commit a release builds and start the
  build), `nightly build`'s `pick`, `sign` and `publish`, `launcher`'s `sign` and `publish`, `nightly command` (it
  writes the set of pull requests the Nightly holds), `native hook stamp` (it pushes a branch). playhorizon is a
  partner's server that the partner can reach, and a checks runner keeps what a fork's job left behind: neither may
  decide or sign what players download. `tests/test_ci_runners.py` pins it.
  Not on GitHub's own runners, as the one exception: [hosted parity](#hosted-parity)'s fetch step, on Blacksmith's,
  holds the vanilla data's two secrets. It decides, signs and publishes nothing, and its token only reads.
- **Release builds**: Linux on `ubuntu-22.04` (players' binaries must link against its older glibc; our Linux
  machines have newer ones), Windows on `windows-latest` (no Windows machine). macOS builds already run on the Mac
  mini.

- **Native Clippy**: `checks` lints `horizon-client --no-default-features`, `horizon-threads`, `horizon-perf-format`,
  `horizon-perf-share` and `horizon-server` on `windows-latest` and `macos-latest`, with all targets and warnings denied.
  These two jobs start only after Linux `verify-lint` passes, skip documentation-only pull requests and run no tests or
  release builds. Their result feeds the required `verify` aggregate: a failure or cancellation blocks it, while a
  skipped job passes. Main and manual runs also lint; main saves the per-OS Rust caches that pull requests read. Hosted
  runners keep untrusted builds away from the Mac mini's game files and signing setup; their tokens are read-only.
  The merge queue skips `verify-lint` and therefore its native jobs too; each queued pull request was already linted.

## verify on Blacksmith

`checks`' `verify` runs on Blacksmith's runners (blacksmith.sh sponsors our minutes) for every pull request, forks'
included, the whole merge queue and main's pushes: ~100 runs an hour on 2026-11-03, more than the CI VPS's two checks
runners could take. They are GitHub's image on faster machines, outside GitHub's 20-job pool, so `verify` is one job
per part, side by side, each on a size that fits it:

| Job | Runner | Merge queue |
| --- | --- | --- |
| `verify (which parts run)` (`verify-scope`): `tools/ci/rust_changes.py`'s answers for the launcher and Rust parts, so a change that doesn't reach a part never starts its runner | 2 vCPU | yes |
| `verify (tree and Python tests)` (`verify-tree`) | 8 vCPU: the Python tests run one module per process (`tools/ci/unittest_parallel.py`) | yes |
| `verify (format and lint)` (`verify-lint`), when `verify-scope` finds a Rust change (always on main's pushes) | 16 vCPU: clippy keeps them busy | no |
| `verify (server build lint)` (`verify-server`), when `verify-scope` finds a Rust change | 4 vCPU: its clippy keeps about two cores busy | no |
| `verify (launcher)` (`verify-launcher`), when `verify-scope` (`rust_changes.py --launcher`) says the change reaches it | 8 vCPU | no |
| `verify (Rust tests)` (`verify-test`), when `verify-scope` finds a Rust change | 16 vCPU: a cold build folder compiles the workspace | yes |
| `verify (mod API)` (`verify-mod-api`), when `verify-scope` says the change can reach the mod API: the comparison and the surface report run on pull requests only; elsewhere it builds the generator to warm its disk | 4 vCPU: the generator and the comparison use one core; only its compiles use more (8 vCPU ran at ~13%) | yes |
| `verify (Windows Wasm cache)` (`verify-windows-wasm-cache`), on pull requests changing `crates/horizon-mod-wasm/`, workspace Cargo configuration, the Rust toolchain pin, the workflow, or the selector | GitHub's `windows-latest`, x64 MSVC | no |

Only `verify` is required, and it waits for all the parts. `SELF_HOSTED_CI=all` sends the Linux parts to our checks
runners. The Windows part runs just the `horizon-mod-wasm` `module_cache` tests on a fresh GitHub runner without a
build cache. `verify` accepts a skipped part when `verify-scope` answers exactly `false` or the event excludes that
part. A failed or cancelled `verify-scope` fails `verify`. The Linux build parts end with a `load probe:` line
(`tools/ci/load_probe.sh`: cores, load, CPU and memory use, seconds) for choosing runner sizes; its `peak` is the job's
cgroup peak, file cache included, so it reads higher than the memory the processes held.

The sizes came from those lines and the step times of 39 pull requests' runs on 2026-11-20. A size halved halves a
minute's price (Blacksmith bills by vCPU, 2 vCPU and 8 GB at a time; 4 vCPU has 16 GB, 8 vCPU 32 GB), and only the
compiles in a part slow down on fewer cores. The sticky disks' keys don't name the size, so a resized part keeps its
build folder.

Each Linux Rust job keeps its Cargo build folder between runs on a Blacksmith sticky disk, as the VPS keeps its own
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
Because no pull request can save a disk, `verify (mod API)` keeps there what pull requests trust: each saved run's
mod API snapshot of its commit (`mod-points-snapshots`, the newest 30, a name kept from the job's earlier name), which a
pull request whose `HEAD^1` is stored compares with instead of building the base
(`tools/check_mod_api_unchanged.py --snapshots`).

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
  the launcher, the Python tests and the whole history's tree check. Not the mod API comparison (pull requests only).
  They mount main's disks and never save them, so a branch near main takes about as long as a pull request: 7 minutes
  on 2026-11-19, the tree check of the whole history the slowest part. The Windows job took 48: 28 for the game, 4 for
  the folder, 15 for the launcher's tests.
- **Windows x64** (`-f windows=true`, `windows-x64`, on `windows-latest` as the release build): the game built as
  `game-build.yml` builds it, uploaded as the artifact `horizon-windows-x64` (kept 3 days): `horizon-client.exe`,
  `horizon-ctl.exe`, `horizon-prepare.exe`, `horizon-server.exe`, `horizon-wasm-std.wasm` and DXC. No exporters (they
  are compiled from Minecraft's jars on the Mac mini), so point it at resources an installed Horizon already prepared
  (`--resources`). Then the Wasm cache and launcher tests run. It builds from scratch, with no Rust cache read or
  saved. The repository is public: anyone signed in to GitHub can download the artifact for its 3 days, so a try run
  is no private channel.
- `-f arm64=true` adds the Windows ARM64 job (about 90 minutes) and `-f jfr=true` the JFR analysis on the Linux game
  runner; every hand-started run ran both before the inputs existed.
- No job of a try run holds a secret; verify's parts and the Windows x64 job save no disk or cache
  (`tests/test_ci_runners.py`). Runs of one commit share a concurrency group: a second waits for the first.
- A try run reports `verify` on the branch's commit, as a pull request's run of that commit does. It is no proof for
  a pull request: it skips the mod API comparison, and the merge queue still needs its own green `verify` on the
  queue's commit, which a try run cannot give.
- The run uses the branch's own copy of `ci.yml`: a branch from before 2026-11-19 has no inputs and always starts the
  ARM64 and JFR jobs. Rebase it on main first.

## Testbox

`testbox.yml` gives a maintainer a Blacksmith runner for building, linting and testing local changes before pushing:
`blacksmith testbox warmup testbox.yml --ref testbox/<name> --idle-timeout 5`. The CLI syncs unpushed code to the
runner and runs commands there over SSH, so the job runs code nobody reviewed. It therefore runs only on `testbox/*`
branches and fails on any other ref: on main or a tag it could write Actions cache entries that release builds
restore, and on a branch another workflow builds (`launcher-*`) entries that a build holding a secret restores.
Cargo's settings (the build folder, mold, `HORIZON_NO_AUDIO`, `HORIZON_TELEMETRY_URL=off`) are in the runner's
`~/.cargo/config.toml`, so they reach only processes Cargo starts. The build folder is a sticky disk of its own
(`-testbox-target`) that is never saved, so each session builds from scratch and stays fast after that: a saved disk
would restore the dates of the checkout over files a session had edited, and the next session could reuse what
someone else's unpushed code built.

A session ends after its idle timeout or at the job's 120-minute limit, whichever comes first. The idle timeout
(`--idle-timeout`) counts minutes with neither a `testbox run` nor an open SSH connection to the runner: a command
still running, a copy over SSH or a forgotten interactive session keeps the session alive, and billed, until the limit.
Billing stops with the session, so the limit costs nothing in normal use and only caps a session that keeps running.
It is 120 rather than 60 because a new session starts with an empty build folder: a cold release build
of the game, the server and the bots took 9 minutes on 2026-11-22, and comparing a branch with its base builds twice.

## Hosted parity

`.github/workflows/parity-hosted.yml` runs vanilla harnesses on Blacksmith's 16-vCPU runners, outside the Mac mini.
A push to main starts it for every harness hosted runners can run, that commit alone (not merged with a newer
main), at most every 3 hours, and a maintainer can start it by hand. It is advisory: no required check waits for
it, and its jobs are named `hosted parity …` so nobody mistakes them for the Mac's `parity / parity`. The job to read is
`hosted parity (advisory)`: it fails when a shard failed, when the plan failed, and when the run had no access to
the data ("hosted parity did not run: no access to the data"). It passes only when every shard passed, or when the
plan had nothing to run.

- **Main's runs, one at a time.** Pushes to main share one concurrency group that never cancels a run in progress.
  Of the runs waiting, GitHub keeps only the newest. A hand-started run has a group of its own. There is no
  schedule: a scheduled event carries no repository object, which the cache gate and the trusted commit read.
- **Spacing.** A push runs in full only when main's last full push run started at least 3 hours ago, about 8 a day.
  Otherwise its first job, `hosted parity spacing`, ends it: `hosted parity (advisory)` passes with one line saying
  when the last full run started and after when the next push runs in full, and leaves no verdict. A full push run
  uploads a `parity-hosted-full-start` artifact (kept a day). The spacing job reads the newest one whose run the
  runs API names as this workflow file's, on a push to main of this repository (another workflow's marker holds
  nothing back), with `actions: read`, the only permission any job has beyond `contents: read`; it checks nothing
  out, restores nothing and holds no secret. What it cannot read skips the run. A hand start (`-f full=true`, as the
  Nightly cut uses) is never spaced.
- **The verdict.** `hosted parity (advisory)` ends with the tested commit, the harnesses planned, passed and failed
  (by name), those that passed only on retry, did not finish, or were left out (with why, as one escaped line), and
  the known failing ones that failed again, passed in every harness or shard they cover (those can come off
  `known_failing` once parity-main agrees, shards left out of the plan included), or passed where run but not in all
  of them. When shards ran, it passes only with every shard's results downloaded and a result for every planned
  harness; otherwise it fails as incomplete, naming what is missing. The same is in the `parity-hosted-verdict`
  artifact (`verdict.json`, kept 30 days) and on one `parity-hosted-verdict {...}` line of the job's log. Main's
  verdict is the run whose `event` is `push` and `selection` is `full`.

```sh
gh workflow run parity-hosted.yml --ref <branch> -f harnesses="food light spawn_loot"   # these, by name
gh workflow run parity-hosted.yml --ref <branch> -f base=<commit>                       # what the branch's changes reach
gh workflow run parity-hosted.yml --ref <branch> -f full=true                           # everything, as main's pushes
```

- **The game's data.** Horizon's code never downloads the game. The data comes from `horizon-rust/vanilla-data`, a
  private repository of the maintainers' own Minecraft files.
  - It holds them encrypted (AES-256, zstd) in datasets made with `lab.py prepare`. Today there is one,
    `26.3-linux-x64`: the jars and libraries, without sources, sounds or other languages.
  - A read-only deploy key (`VANILLA_DATA_DEPLOY_KEY`) clones it, and `VANILLA_DATA_PASSPHRASE` decrypts it.
  - Those two secrets reach the step that runs `tools/ci/fetch-vanilla-data.sh`, and the passphrase also reaches
    the steps that run `tools/ci/sealed-cache.sh` (below). The scripts write them to a private folder, remove that
    folder before they end, and print only sizes, counts and timings.
  - Fetching and unsealing run before the tested commit is checked out, sealing after the harnesses on main only.
    Each of those steps takes its script from the GitHub API at the default branch's commit (resolved through
    `heads/<branch>` by the plan job, which restores nothing) into a new private folder, checks that it is not empty
    and that its git blob id is the one the API names for that path (asked for separately), and runs it under
    `/usr/bin/bash` with `env -i`, `PATH=/usr/bin:/bin`, an empty `HOME`, no `BASH_ENV`/`ENV`, and the secrets on
    standard input. Nothing a secret step executes or reads as code comes from the workspace, `.git` or a restored
    path. Without the fetch script, or with one that fails the check, the shard stops; without `sealed-cache.sh` it
    starts cold and saves nothing. The workflow file itself comes from the dispatched ref, so who may dispatch
    (write access) is still the guard against a branch that edits it.
  - What this does not stop: a restore extracts with absolute paths, so a hostile entry can write any file the
    runner user owns, among them the code of a later action step (the cache action, node). Code run that way
    shares the secret steps' user and could watch them. Branch-scoped cache entries (GitHub's rule, and
    Blacksmith's default, which the owner confirms in its dashboard) are what keeps such an entry from a run of
    main.
  - Only a commit on a branch of this repository runs, resolved once to a full SHA that every job uses.
- **Caches.** A fork's pull request can restore this repository's cache entries, and a branch's copy of the workflow
  can name any key. So only a run of main writes them, and what is made from the data goes into one only sealed.
  Every other run restores and saves nothing. No sticky disk, no sccache.
  - The plan job's `saves` is true only for a push, schedule or dispatch whose ref is exactly `refs/heads/main`, is
    the default branch and is protected, compared in shell (expressions compare strings without case). It is a job
    output, not a variable: the oracle shim keys every variable the harnesses see, so a variable that differs
    between main's saving run and a reading run would make every reading run miss.
  - The dataset's encrypted chunks, as they are. The fetch script takes them only when they are plain files that
    match the MANIFEST it reads from `horizon-rust/vanilla-data` each run, and downloads the dataset otherwise.
  - Sealed by `tools/ci/sealed-cache.sh`: the build folders (`.local/parity-target`, `target/`, one entry per probe
    group, keyed by the toolchain and the UTC week), the prepared resources, and the oracle results with the
    parity state. Sealing is tar, zstd, the dataset's own encryption (AES-256-CBC, PBKDF2), then HMAC-SHA256 with
    a second key derived from the passphrase, over the entry's kind, its full cache key and the ciphertext.
    Sealing takes folders and regular files only: symbolic links stay out (each run makes its own again: the
    prepared resources' links to the data, the shim's `java` and `javac`), a folder given as one included.
    Unsealing checks the tag and that the key has the prefix the run asked for, decrypts, then lists the archive. It
    refuses absolute or `..` names, a name twice, any symbolic link, a hard link other than to a regular file
    listed before it by exact name (Cargo's binaries), and special files, then extracts into a new empty folder
    with `--keep-old-files`, renamed into place only when whole. An entry that fails is discarded with a warning,
    and the run goes on cold. The build is sealed too because a build script could copy data into it, and so that
    the binaries the harnesses run are authenticated.
  - Incremental builds are on (`CARGO_INCREMENTAL=1`) and `tools/ci/source_mtimes.py` gives unchanged sources their
    old dates, so a change rebuilds only what it touches. A damaged date file, oracle record or hash database is a
    miss, never a red job. A shim that meets a failing hash database hashes files itself and never removes it (another
    may have it open). Here, `tools/parity.py` removes a damaged one, found by `PRAGMA quick_check`, before any
    shim starts (`PARITY_REPAIR_ORACLE_HASHES=1`: the runner's one parity run owns the cache); elsewhere it only
    names it. An oracle record holds the length and SHA-256 of its output streams, which are written
    before it under temporary names: a stream cut short or of another write is a miss, never replayed. The oracle
    shim's runtime memo (`.local/parity-runner/runtime-memo`, guarded by one machine's file stat data) is never
    sealed or restored.
- **What never leaves the runner** unsealed: the data, what is prepared from it, and `.local/` but the build. None
  of it goes into an artifact.
- **What is public.** The job logs and the uploaded logs hold what `parity.yml`'s public runs already show: compared
  values, log tails and harness names. That is the only exception to "sizes, counts and timings only", which the
  fetch step keeps. The upload holds the first run's log of each retried harness, the last log of each that failed,
  and how each retry ended (from its saved record), kept 3 days. `tests/test_ci_runners.py` pins all of this.
- **Plan, shards, result.** A `plan` job, with no secret and no data, selects with `tools/parity.py` and splits the
  selection with `tools/parity_plan.py` into shards by the probe each harness builds (world, client, game, other), so
  a runner pays only that group's cold build.
  - There are no hosted dependency lists yet, so with `base` every harness counts as reached, and the full set is
    planned. The lists will come from a hosted full run on main.
  - `base` matters only when the head conflicts with main: otherwise the head is tested merged into main's tip and
    compared with it, as in `parity.yml`.
  - Each shard runs on a `run` runner with Xvfb and Mesa's software Vulkan (lavapipe), as playhorizon has. The oracle
    cache is created before the harnesses start, since shims that create it at once can fail.
  - Each shard uploads `results-<shard>.json`: whether each harness passed and whether it was retried, from the
    records its own run finished (the restored state holds main's earlier ones). A harness without one, a shard
    stopped mid-run or mid-retry included, is listed as not finished, never as passed.
- **Not run on hosted runners.** These are listed with their reason, never counted as passed:
  - the harnesses the `hosted` entry of `tools/parity_hosts.json` excludes: sounds, other languages, sources, older
    servers' worlds, the network;
  - `named_only` and `local_only` ones, and `known_failing` ones except in a full run.
- **A full run (main's pushes), 2026-11-23.** 659 harnesses (17 of them known failing; 42 left out), in 14 shards
  of about 50, then about 20-30 min a shard (a 50-harness shard took 29 min on 2026-11-22): about 280-420 minutes
  of 16-vCPU runner time and 30-35 min of wall time per run. With the spacing, about 8 a day: about 2,200-3,400
  16-vCPU minutes a day, plus a minute or two of 2-vCPU time for each push that is skipped.
- **Cache entries by the week.** A key is never overwritten, and the runners' cache can be pruned only from its
  dashboard or with an account token no workflow holds. So the build and oracle keys name the UTC week instead of
  the commit: main's first full run of a week saves the build (about 15 GB) and one oracle entry per shard (about
  0.2 GB each); later runs that week restore them and save nothing. Last week's entries go unused and are evicted 7
  days after their last use: about 18 GB current, up to about 36 GB in the week after a change of week. During a
  week, a run rebuilds what changed since the week's first run (Cargo's fingerprints, with the sources' dates).
- **Cost, measured 2026-11-19 (cold).**
  - About a minute of setup and data per runner.
  - Cold probe builds: world 40 s; client 530 s (two profiles); the game 480 s.
  - Then 3-10 s of runner time per harness, 8 at a time, most of it in the vanilla oracle.
  - From main's caches, the builds take seconds, or what a change touches. The cached state serves the probes'
    prebuild and the oracle results' reuse, not the selection: the plan job holds no secret, and its dependency
    lists are still to come from a hosted full run on main.

### The fuzzer's workflow

`.github/workflows/parity-fuzz.yml` runs the scenario fuzzer (docs/parity-workflow.md, "Fuzzing scenarios") on the same
runners, started by hand only (`gh workflow run parity-fuzz.yml --ref <branch> -f family=commands -f seed=1 -f count=200
-f runners=2` for a wide search, or `-f seeds=1017,1325 -f shrink_rounds=4` for those seeds again, one slice each,
shrunk). It takes the data by the rules above and adds none:

- **Same access.** Only a commit on a branch of this repository; the fetch script comes from the default branch's
  commit; the one step that holds a secret is hosted parity's fetch step, line for line, and ends before the first
  checkout. It is a copy, not a shared step: an action or a reusable workflow would be one more file whose ref a
  run has to trust. `tests/test_ci_parity_fuzz.py` compares the two files instead, so a change to one that is not
  made in the other fails. Without the secrets the plan job fails: no green run that fuzzed nothing.
- **No cache.** No job restores or saves a cache entry, and the file says `cache-mode: none` for a cache service
  that honours it. A hand start may write to its branch's cache scope, so an entry restored before the step with a
  secret could be one the same run's build job wrote. Each fuzz job therefore downloads the dataset and prepares
  the resources itself; the sealed entries stay hosted parity's.
- **No Cargo beside the data.** The game and the preparer (`horizon-prepare`) are built once, in a job with no
  secret, no data and no cache. The fuzz jobs run those two programs (`HORIZON_PREPARE_BINARY` tells
  `tools/ci/vanilla-data.sh` to use the built preparer), so no build script of the tested commit runs on a runner
  that holds the data. The tested commit's game, preparer and Python tools do run there, as in hosted parity.
- **What leaves the runner.** Each fuzz job uploads a folder made new after its slices ended: each slice's output,
  and `found.json` and `report.json` (compared values and commands, as hosted parity's logs) parsed as JSON and
  written again. Only regular files are read, through no link (`staged` in `tools/parity_fuzz_probe.py`). A
  step that runs only when the slices step ran checks that folder against the exact names the slices step
  writes, and against the digest the slices step gives as its output once it has staged (also when it then fails for a lost slice); the upload runs only when that check passed and found results, so never a folder the slices step did not finish. The summary job uploads its merged
  table. All are kept 3 days.
- **Cost.** One 16-vCPU runner per 8 slices, plus one for the build.

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
