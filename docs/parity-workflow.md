# Parity workflow

How parity work is run, checked and merged. The harnesses themselves are described in [Rust rewrite status](rust-parity.md).

## Running the vanilla harnesses

`tools/parity.py` runs the `tools/test_*_parity.py` harnesses in parallel and reruns only what a change can affect:

```sh
python3 tools/parity.py              # harnesses whose inputs changed, or that never passed
python3 tools/parity.py food light   # these harnesses, whatever their state
python3 tools/parity.py --failed     # the ones that failed last time
python3 tools/parity.py --all        # everything (about 20 minutes on ludodesktop)
python3 tools/parity.py --list       # what would run
python3 tools/parity.py --status     # last result of each harness
```

Each harness runs unmodified under a tracer that records the files it depended on: its Python modules, its Java oracle and the oracle sources javac pulled in, other repository files it read, and the Rust sources of the probes it ran (from Cargo's `.d` dep-info files). A pass stands until one of those files, the pinned vanilla artifacts, `Cargo.lock` or the Rust/Java toolchains change. State and logs are in `.local/parity-runner/`; each harness's own outputs stay in its `.local/<name>/` directory.

Probes build into `.local/parity-target/` with release optimisation but no LTO and 16 codegen units. On ludodesktop that rebuilds every `horizon-world` probe in 3 seconds after a one-line change, against 122 seconds with the release profile. Rust gives these settings identical results (no fast-math; overflow checks as in release), and the full harness suite passes under them. Harnesses must locate binaries through `cargo metadata` or `lab.cargo_target()`, never a hard-coded `target/`.

Parallel harnesses are admitted by memory: each harness's recorded peak is reserved, within three quarters of the memory available at the start. `-j` caps the count (default: a third of the CPU threads).

## The gate

`tools/gate.py` is what a branch must pass before it is integrated: the public-tree check, rustfmt, clippy with warnings denied, the Rust and Python tests, then the harnesses whose inputs changed. It stops at the first failure and writes `.local/gate.json`. `--quick` checks formatting, lints and Python tests only; `--no-parity` skips the harnesses.

## Parallel work: worktrees and the integration branch

Parity work happens in one worktree and branch per area, next to an integration worktree that has the integration branch (`parity-base`, pushed as `t3/achieve-full-parity`) checked out. All worktrees share the vanilla data through `.local/` links to a `shared-local/` directory beside them.

```sh
python3 tools/supervise.py status                       # branches, progress, unpushed work, last gate
python3 tools/supervise.py new dispensers               # new worktree and branch from the integration branch
python3 tools/supervise.py save                         # commit and push unfinished work before agents stop
python3 tools/supervise.py integrate parity/dispensers  # merge, gate, push; undo the merge if the gate fails
```

An agent working an area merges the integration branch first, uses `tools/parity.py` on its harnesses while it works, runs `tools/gate.py` before its last commit and pushes its branch. Integration happens only through `supervise.py integrate`, so the integration branch always passes the gate.

On ludodesktop, source `~/code/horizon-parity/env.sh` first: it sets the ALSA build path and the Cargo job cap. Debug builds carry line tables only (`[profile.dev]` in `Cargo.toml`), which keeps backtraces readable while keeping each worktree's build small; a fresh worktree builds `horizon-client` and its tests in under a minute.

## Reading vanilla

Besides the Vineflower sources in `.local/minecraft/<version>/sources/`, `tools/rea_vanilla.py` queries the shipped bytecode through REA (callers through lambdas, overloads, a second decompiler); see [decompilation status](decompilation-status.md#querying-the-shipped-bytecode-with-rea).
