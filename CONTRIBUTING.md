# Contributing to Horizon

Thanks for helping. Horizon is Minecraft: Java Edition 26.3 rewritten in Rust, and the one rule that shapes
everything is that **it plays like vanilla**: first exactly like it, then better where players gain from it
([vanilla first, then better](#vanilla-first-then-better)). This page is the short version of how we work; questions are
welcome on [Discord](https://discord.gg/playhorizon).

This repository holds the game and the launcher, plus the tools, tests and docs that build, check and release them.
The website (playhorizon.dev) is built in a separate repository from this one's README, `docs/api/` and roadmap. Anything
else (videos, Discord and other community material) lives elsewhere.

**Where to start: [playhorizon.dev/hub](https://playhorizon.dev/hub).** It shows what is being done now and next
([ROADMAP.md](ROADMAP.md) in the repository), the work board, good first tasks, and the forms to report a bug or
suggest something. A pull request for a roadmap item carries its `roadmap:<id>` label; ideas go to the suggestion
queue (the hub, or `/suggest` on Discord) rather than to issues, which are for bugs.

**Mod-related pull requests are opened here, like every other.** That covers the mod API, native hooks and events
for mods, native mods, content mods (`.hmod`), mixins, wasm mods, `cargo horizon-mod`, `horizon-mods`, mod builds and
the registry, and their docs ([mods](docs/mods.md), [native mods](docs/native-mods.md)).

## Build it

- **Prerequisites:** Rust (stable), Python 3.11+, JDK 25, git 2.38 or newer, and on Linux the ALSA and udev headers
  (`libasound2-dev libudev-dev`; Void Linux: `base-devel pkg-config alsa-lib-devel eudev-libudev-devel`).
- **Resources:** Horizon never downloads the game; it prepares its resources from your own copy of Minecraft 26.3, as
  described in [Build from source](docs/native-client.md#run-from-a-checkout).
- **Commands:**

  ```sh
  cargo run --release -p horizon-prepare -- --prepare --server-jar ~/Downloads/server.jar
  cargo run --release -p horizon-game
  ```

- **Launcher:** its own Cargo workspace in `launcher/` (`cd launcher && cargo run`; on NixOS, `nix-shell` first).

## Vanilla first, then better

Horizon gets there in two stages. First it plays exactly like vanilla; that is what parity work is for. Then it
improves on vanilla where players gain from it, mostly speed: FPS, TPS, loading, memory.

Wherever vanilla has a feature, vanilla is the specification, down to the order of random calls. "Parity" here means
Horizon gives the same results as the pinned 26.3 client and server for the same inputs, checked by a **harness**: a
`tools/test_*_parity.py` script that runs a small Java oracle against the real game and a Rust probe against Horizon,
and compares their outputs.

- **Read the vanilla code before writing Horizon's.** `tools/rea_vanilla.py` queries the pinned game bytecode:
  `python3 tools/rea_vanilla.py method net.minecraft.world.level.block.DispenserBlock dispenseFrom`.
  Say in your pull request which vanilla classes and methods you followed.
- **One version: 26.3.** Parity work targets 26.3 until its parity is good, redstone first. Don't open pull requests
  that move Horizon to a newer Minecraft version for now; to join servers on a newer version, put a Via proxy
  (ViaVersion, ViaProxy) in front of them.
- **Ports of vanilla behaviour come with a harness**, new or extended, so the next change can't silently break
  them. [The parity workflow](docs/parity-workflow.md) shows how harnesses run; copy a nearby one. Where vanilla
  has nothing to compare (Horizon's own features, performance work, refactors), no harness is needed: one Rust
  test of the changed behaviour, or the numbers before and after.
- **Horizon's own features** (the launcher, the new menus) follow [the design system](docs/design-system.md) and are
  off by default wherever they would change the vanilla look.
- **Three layers: engine, platform, Horizon.**
  - Vanilla code never depends on the platform (mods, shader packs, HDR, the settings system) or on Horizon
    (its extras, cosmetics, the shop, official servers, telemetry).
  - The platform never depends on Horizon.
  - Horizon uses only the platform, never the engine: it will live in its own repository.

  [docs/layers.md](docs/layers.md) says where a new feature goes; CI checks the dependencies.
- **Everything players run is Rust.** Python and Java are fine for dev tools and harness drivers in `tools/`, never
  for player-facing code.
- **No game files in the repo.** Never commit game files, decompiled source or prepared resources (`.local/`):
  `tools/check_public_tree.py` rejects them (`git config core.hooksPath .githooks` runs it before every commit).
  The pre-push hook checks the full history reachable from the branches and tags being pushed. Run
  `python3 tools/check_public_tree.py --history` to audit the history of all local refs.

### When it doesn't have to be 1:1

Parity is about what players and the vanilla game see, not about copying vanilla's internals. Performance work (a
multithreaded server tick, rendering, chunk loading, memory) doesn't have to keep vanilla's internal order or its
exact random calls. It has to work together with normal Minecraft:

- **Vanilla clients and servers still work with it.** A vanilla 26.3 client plays on a Horizon server, Horizon joins
  vanilla servers, and saves and worlds open in both, as before.
- **The game still plays the same.** Farms, redstone, mobs and drops work the way players know from vanilla. A
  difference may only show where no player can rely on it, such as which of two mobs ticks first or exactly which
  random number a mob gets.
- **Vanilla's slow paths aren't copied.** When vanilla spends a lot of FPS or TPS on something that can reach the
  same result faster, Horizon does it faster. Copying a performance problem isn't parity.
- **Say what changes.** The pull request names the vanilla behaviour it no longer reproduces exactly, why players
  won't notice it or why it's worth it, and how it was checked: numbers before and after, and the harnesses that
  still pass. A change players could notice, such as a different tick order on a busy server, goes behind a setting.

## Vendor libraries and Java mods

- **Upscalers and latency tools.** AMD FSR (MIT) may go in the repository.
  NVIDIA's DLSS, Reflex and any other library under an NVIDIA license may not: they ship only as an optional native mod
  that downloads NVIDIA's files itself, on the player's machine. No NVIDIA binaries, headers or SDK files in the
  repository.
- **The JVM bridge** (running Java mods) is only ever a separate, optional download, never installed or on by
  default. The game may offer to turn it on when it finds a Java mod; the player decides. It is promoted out of beta
  only after MSPT and RAM benchmarks against the Java client with the same mods ([#13914](https://github.com/horizon-rust/horizon/issues/13914)).

## Licence

Horizon is under the GNU Lesser General Public License, version 3 or any later version ([LICENSE](LICENSE)). The
LGPL adds permissions to the GNU GPL version 3, whose text is in [LICENSES/GPL-3.0.txt](LICENSES/GPL-3.0.txt). What
you contribute is under the same licence. Code merged before the move from MIT stays available under the MIT licence
([LICENSES/MIT.txt](LICENSES/MIT.txt)).

Code taken from another project must be under a licence the LGPL-3.0 accepts, such as MIT, BSD, ISC, zlib,
Apache-2.0, MPL-2.0, LGPL-3.0 or LGPL-2.1-or-later. It keeps its copyright line and licence notice, and gets an entry
in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md). Code under the GPL or the AGPL can't go in: it would put all of
Horizon under that licence. If you aren't sure, ask on Discord first.

## Before asking for review

Run the self-check and paste the block it prints into your pull request:

```sh
python3 tools/pr_ready.py --base upstream/main   # or origin/main if you work on the main repository
```

It looks at what your branch changes and runs what matters: rustfmt, clippy with warnings denied, the tests of the
crates you touched (and the launcher's), generated files, `Cargo.lock` and the Python tests. Like CI, it runs the
vanilla harnesses only when your branch changes a harness, an oracle, a probe or the runner; otherwise it lists the
ones your change can affect, and main's parity run checks them after the merge. `--harnesses` runs them on your
machine anyway (it needs the vanilla data). It exits non-zero when something fails.

For the game workspace, Clippy checks all targets of the touched crates and their transitive workspace dependents,
including examples. The dependency scope includes optional, platform, build and development dependencies. Tests run
library, binary, integration and doc tests without building unrelated example executables. Shared Cargo
configuration and toolchain changes select the whole workspace; CI also keeps its workspace-wide checks.

For a quick check while you work, `python3 tools/check_changed.py` runs rustfmt, clippy and tests for only the crates your branch changed and the crates that depend on them (`--dry-run` lists the commands).

### Translations

Most of Horizon's translations were written by machine and stay marked unreviewed until someone who speaks the
language checks them. To report a wrong string or review your language, see [docs/translations.md](docs/translations.md)
and #translations on Horizon's Discord. Never mark machine-written text as reviewed.

## Claim before you start

Work is tracked in GitHub issues, and an issue that is assigned is taken (so is one with the `in progress` label:
claimed on Discord with Kira's `/claim`): pick another, or ask its holder in the thread. Before you start, comment
**`/claim`** on the issue; you are assigned at once, no role needed. No issue for your work yet? Open one (a line is
enough), then claim it. `/unclaim` gives it back. Issues labelled `tracker`, `proposal` or `not claimable` aren't
claimed whole: claim one of the issues a tracker lists, or open one for the part you take. For the first hour after
an issue is filed only its author can claim it, so whoever found a problem gets to fix it (issues filed by bots have no such hour).

A claim with no open pull request linking the issue (`Fixes #123` in the description, or a draft) is released after
5 days, with a comment: open a draft early. You can hold 8 open issues at a time, not counting the ones your
pull requests already fix. Agents claim the same way.

When your pull request changes the same functions as another open one, or closes the same issue, a bot comment names
it (edited on every push, never repeated): talk to its author in the thread before you both finish the same work.

## Pull requests

- **One change per pull request**, under about 3000 lines of code; split bigger work into pieces that each make
  sense alone. Smaller pull requests get reviewed and merged faster. Bigger than that:
  [large pull requests](#large-pull-requests). What counts: lines added plus lines removed, outside Markdown (`.md`)
  files and test files (any `tests/` or `test/` folder, and `test_*.py`). Tests inside a source file
  (`#[cfg(test)]` modules) don't count, and neither do lines moved unchanged (same text, whitespace ignored, runs of
  3 or more lines), so a refactor that moves code can be far bigger than 3000 lines.
  pr_ready counts it the same way and warns when a pull request is over.
- **Small fixes of one kind are one change.** The same defect in several tools, or several small fixes to one tool
  or harness, go in one pull request that lists each issue it fixes (`Fixes #1, Fixes #2`).
- **Fill in the template**: what and why, the vanilla reference, the pr_ready block, screenshots for anything you
  can see.
- **Prove an in-game change before you open the pull request.** If players would see or feel the change in the
  game (gameplay, mobs, blocks, items, rendering, UI, multiplayer, saves), the description shows it working on the
  pull request's own head commit: what was wrong, what you did in the game to check it, and what happened. Attach
  before and after screenshots or a short video, or the `lan_e2e`, harness or `horizon-ctl` commands you ran with
  their output. A unit test alone doesn't count, and neither does a proof from an older commit. Without it the pull
  request gets `needs-proof` and waits for review until the proof is there. Tooling, docs, tests and refactors that
  change nothing in the game don't need it.
- **Tick "Allow edits by maintainers".** When something small is left (a lint, a rebase, a nit) we push the fixup
  ourselves instead of sending it back to you; only design questions come back.
- **Once we've pushed to your branch, merge main into it; don't rebase or force-push.** A rebase or force-push
  replaces our commits with yours and erases the fixes we added (it happened on #5188 and #5203). Tell your agent
  too: `git merge origin/main`, never `git rebase` or `git push --force` on a branch a maintainer has touched. If
  our fixes get erased anyway, we finish the work in our own pull request that keeps your commits and credits you.
- **Title: `kind(area): what changes`**, so the queue reads at a glance: `parity(mobs): a ghast's fireball uses its
  ExplosionPower`, `perf(launcher): download the game's files in parallel`, `bug(saves): keep /attribute modifiers
  through a save`. The kind is one of the words below; the area is an area label, several with commas
  (`perf(chunks,rendering): ...`), or leave it out (`docs: ...`). Release notes drop the prefix.
- **Tick its kind** in the template: `parity` (Horizon now does what vanilla does), `bug` (a crash or breakage),
  `perf`, `feature` (new for players), `extra` (Horizon-only, off by default), `tooling` or `docs`. A workflow sets
  that label from the box, else from the title, and area labels (`launcher`, `mobs`, `worldgen`, `rendering`...)
  from the title and the paths you change. A size label (`size: XS` up to 50 lines, `S` 150, `M` 500, `L` 3000,
  `XL` over) counts the lines GitHub shows as added and removed outside Markdown and test files: what counts above,
  but with moved lines and inline tests, which the label can't tell apart.
  `label:parity` is the list of real progress toward vanilla.
- **Release labels** (we set them if you can't): `feature` also puts it under "New" in the release notes,
  `skip-release` when players see no change (tooling, docs, tests), nothing for a fix. `release-now` is for crash
  fixes that shouldn't wait. pr_ready suggests one.

Who reviews and merges: the Horizon Team, on CI for ordinary pull requests; an in-game change waits for the proof
above, and risky ones (saves, network, world generation, the launcher's updates) also for a local proof from us
([merging](docs/release-flow.md#merging)).
A bot, CodeRabbit, also comments when a pull request opens. Treat it like any reviewer: fix what's right, say why
when it isn't, and resolve the thread. It can't block a merge; `@coderabbitai review` asks it to look again.

Every merge to main is released automatically ([release flow](docs/release-flow.md)), so main always stays playable.
How agents, checks, releases and signed updates keep this safe: [security](docs/security.md).

### Large pull requests

3000 lines of code ([what counts](#pull-requests)) stays the default. A pull request that can't be split below it
starts from a spec:

1. **Open an issue with the spec** before the code: what changes, the files it touches, the behaviour players and
   other code will see, and the tests or harnesses that prove it.
2. **A maintainer approves the spec** in the issue. Code written before that may have to change or wait.
3. **The pull request links the approved issue** (`Spec: #123` in the description) and stays within it; a change of
   plan goes back to the issue first.
4. **An agent check compares the code with the spec**, run by the maintainers' agents: anything the spec doesn't
   cover, or that it promises and the code doesn't do, comes back to you.
5. **Then the normal review**, as for any pull request.

## Parity checks on forks

The `verify` check (formatting, lints, tests) runs on every pull request. The `parity` check runs the affected
harnesses on a maintainer's machine, which holds the game files, so it never runs a fork's code on its own: a
maintainer reads your change, then adds the `run-parity` label, which runs it once on your current commit. After
new pushes, we add it again.

## Talk to us

[Discord](https://discord.gg/playhorizon) for questions and ideas, [issues](https://github.com/horizon-rust/horizon/issues)
for bugs. To take something on, `/claim` its issue ([above](#claim-before-you-start)).

How Horizon is run, and how people join the Team or lead a group: [GOVERNANCE.md](GOVERNANCE.md).
