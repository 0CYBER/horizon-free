# Contribute

Horizon is open source and takes pull requests. The one rule that shapes everything: **it plays exactly like vanilla.** Wherever vanilla has a feature, vanilla is the specification.

Questions and ideas are welcome on [Discord](https://discord.gg/HcTeMZxe5E). Bugs and feature requests go in [GitHub issues](https://github.com/ludovic111/horizon/issues). If you want to take something on, say so in its issue so nobody else starts the same work.

## The repository

[github.com/ludovic111/horizon](https://github.com/ludovic111/horizon) holds the game, the launcher and this website, plus the tools, tests and docs that build, check and release them.

- `crates/` is the game, as Rust crates: the client, the server, the world and protocol code, the mod system, the design tokens and more.
- `launcher/` is the launcher, a GPUI app with its own Cargo workspace.
- `website/` is this site and its documentation, in `website/docs/`.
- `tools/` holds the parity harness drivers, the parity runner, the release tools and the self-check.
- `tests/` holds the Python tests, `benchmarks/` the published results and `packaging/` the installers.
- `docs/` holds the developer documentation, [indexed here](https://github.com/ludovic111/horizon/blob/main/docs/README.md): the native client, the design system, world generation, the parity workflow, the release flow and more.

## Build it

You need Rust (stable), Python 3.10 or newer, JDK 25 and, on Linux, the ALSA and udev headers (`libasound2-dev libudev-dev`). Horizon never downloads the game: it prepares its resources from your own copy of Minecraft 26.3.

```sh
git clone https://github.com/ludovic111/horizon.git
cd horizon
cargo run --release -p horizon-prepare -- --prepare --server-jar ~/Downloads/server.jar
cargo run --release -p horizon-client
```

Everything prepared lands in the git-ignored `.local/`. It is derived from the game: never commit it or share it. The launcher is its own workspace: `cd launcher && cargo run`. The website builds with `node build.mjs` in `website/`.

## Parity harnesses

A **harness** is a `tools/test_*_parity.py` script. It drives the same inputs through Horizon and through the vanilla client and server bytecode from your own copy of the game, and checks that the outputs are identical. There are {{HARNESSES}} in the audit.

- **Read the vanilla code before writing Horizon's.** `tools/rea_vanilla.py` queries the pinned game bytecode, for example `python3 tools/rea_vanilla.py method net.minecraft.world.level.block.DispenserBlock dispenseFrom`. Say in your pull request which vanilla classes and methods you followed.
- **Gameplay changes come with a harness**, new or extended, so the next change cannot silently break them. Copy a nearby one.
- `python3 tools/parity.py` runs the harnesses a change can affect. [The parity workflow](https://github.com/ludovic111/horizon/blob/main/docs/parity-workflow.md) explains the rest.
- Horizon's own features (the launcher, the new menus) follow [the design system](https://github.com/ludovic111/horizon/blob/main/docs/design-system.md) and stay off by default wherever they would change the vanilla look.
- Everything players run is Rust. Python and Java are fine for dev tools and harness drivers, never for player-facing code.

## How pull requests are checked

1. **Self-check first.** Run `python3 tools/pr_ready.py` and paste the block it prints into your pull request. It runs rustfmt, clippy with warnings denied, the tests of the crates you touched, the Python tests, the public-tree check and the harnesses your change can affect.
2. **`verify`** runs on every pull request on GitHub's Linux runners: the same checks, without the vanilla harnesses. Docs-only changes finish in seconds.
3. **`parity`** runs the affected harnesses on a maintainer's computer, which holds the game files. It never runs a fork's code on its own: a maintainer reads the change, then adds the `run-parity` label, which runs it once on your current commit.
4. **Small pull requests.** One change each, under about 500 lines of code where you can. They get reviewed and merged the same day.
5. **Labels.** `feature` when players get something new, `skip-release` when they see no change, nothing for a fix. We set them if you cannot.
6. **"Allow edits by maintainers."** Tick it. When something small is left, we push the fix ourselves instead of sending you back.

Every merge to `main` that players can see is released automatically, so `main` always stays playable. See [the release flow](https://github.com/ludovic111/horizon/blob/main/docs/release-flow.md).

## What never goes in

Never commit game files, decompiled source or prepared resources. `tools/check_public_tree.py` rejects them, and `git config core.hooksPath .githooks` runs it before every commit. Tracked Java is allowed only under the project's own `dev/horizonlab` namespace.

[CONTRIBUTING.md](https://github.com/ludovic111/horizon/blob/main/CONTRIBUTING.md) is the short version of this page, and the [contributors](/#contributors) are listed on the home page.
