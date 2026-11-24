# Paper event parity

`tools/paper_parity.py` runs the same fixed-seed scenario against Paper 26.3
(a logging Java plugin) and Horizon (a native trace mod), driven by `horizon-bots`.
It compares observable events, cancellation and resulting state where covered.
Hook → event mappings come from `paper = "..."` attributes in
`crates/horizon-mod-vocab/src/hooks/`, not a second hand-maintained mapping.
This is a porting aid, not a Bukkit compatibility layer or a replacement for
the vanilla [parity harnesses](parity-workflow.md). It never edits ledger rows.

## Scenarios and tolerances

The executable contract is [paper_parity_scenarios.json](../tools/paper_parity_scenarios.json).
`tools/paper_parity.py` supports three scenarios (`chunk_load`, `hotbar` and `moisture`),
allow/cancel runs, `--scenario` and `--startup-timeout`.

| Scenario | Compared observations | Tolerance |
| --- | --- | --- |
| `chunk_load` | `chunk_load` / Paper chunk-load event; `(dimension,x,z,new)` | Set equality, ignoring tick, ordering and duplicates. Includes server startup/spawn warm-up through the bot's quit; no spatial exclusions. |
| `hotbar` | `player_switch_hotbar_slot_pre`; requests 3 → 7 → 3, cancellation of 7, selected slots and Horizon post events | Exact `(old,new,cancelled)` sequence; relative ticks versus script and between servers have spread ≤1. Cancellation keeps slot 3, making the last request a no-op. Post fires only on accepted changes. |
| `moisture` | `moisture_change`; dry/wet farmland, cancellation and saved final blocks | Set of `(pos,from,to,cancelled)` over 400 server ticks at random-tick speed 1024 after 40 settling ticks. Ignore event ticks and repeated cancelled attempts. Dry ends as dirt or naturally spread grass; wet/cancelled stays moisture 7. |

## Run locally

Install stable Rust, Python 3.11+ and JDK 25 (`java` and `javac` on PATH).
On Linux install the build prerequisites in [CONTRIBUTING](../CONTRIBUTING.md).
Use your own Minecraft 26.3 installation and server jar: `lab.py prepare`
imports and checks them against `research-lock.json`; it does not download Mojang's game.
See [the parity workflow](parity-workflow.md#reading-vanilla) for local vanilla data setup.

From the repository root, on Linux:

```sh
python3 tools/lab.py prepare --server-jar /path/to/server.jar
cargo run --release --locked -p horizon-prepare -- --root "$PWD"
export CARGO_TARGET_DIR="$PWD/.local/paper-parity/target"
python3 tools/prepare_paper_trace_engine.py
cargo build --release --manifest-path .local/paper-parity/engine-project/Cargo.toml
cargo build --release --locked -p horizon-bots
python3 tools/paper_parity.py run --engine "$CARGO_TARGET_DIR/release/paper-trace-engine" --bots "$CARGO_TARGET_DIR/release/horizon-bots"
```

On Windows (PowerShell, from the repository root):

```powershell
python tools/lab.py prepare --server-jar C:/path/to/server.jar
cargo run --release --locked -p horizon-prepare -- --root "$PWD"
$env:CARGO_TARGET_DIR = Join-Path $PWD '.local/paper-parity/target'
python tools/prepare_paper_trace_engine.py
cargo build --release --manifest-path .local/paper-parity/engine-project/Cargo.toml
cargo build --release --locked -p horizon-bots
python tools/paper_parity.py run --engine "$env:CARGO_TARGET_DIR/release/paper-trace-engine.exe" --bots "$env:CARGO_TARGET_DIR/release/horizon-bots.exe"
```

Set `CARGO_TARGET_DIR` before staging: the script installs the trace fixture beside
the engine there. The default resources are `.local/horizon-client/26.3`;
use `--resources /absolute/path` to override. `run` runs all scenarios by default;
`--scenario hotbar` selects one scenario and `--startup-timeout 2400`
allows slower server startup on network/slow disks (default 1500 seconds).
Prefer a local SSD for resources, Cargo output and `.local/`.

Paper is fetched by `server_bench.paper_jar` into `.local/`, with SHA-256 checking.
The harness seeds Paper's Mojang cache from the imported pinned jar. Each server
gets a scratch game directory and fresh Horizon/launcher/XDG homes, audio disabled
and launcher self-updates disabled; runs do not use real player saves or settings.
Never commit or upload jars, prepared resources or world directories.

## Read a report

The command prints the output directory and per-hook `match`, `mismatch: <reason>`
or `not covered` lines. Uncovered means no scenario/normalizer covers that hook,
not a passing comparison. Inspect `paper.jsonl`, `horizon.jsonl`, `run.json`,
bot JSON summaries and server/bot logs for a mismatch. Rerun just the comparison:

```sh
python tools/paper_parity.py report .local/paper-parity/<printed-directory>
```

`tools/paper_parity.py run` writes `run-*` directories with child
scenario directories and an aggregate `run.json`. Keep local worlds if you need
to repeat the moisture final-block checks.
The CLI returns 0 for matches and 1 for mismatches, including caught harness errors.

## Known differences

- At view distance 2, Paper's full-chunk radius is 5, one ring larger than
  vanilla/Horizon's radius 4. The chunk-load comparison retains these border
  chunks and reports the difference; it does not hide the outer ring.
- Horizon keeps spawn Y when candidate columns are unlit
  (`game/world/initial_spawn.rs:346–359`). Different spawn origins would invalidate
  the shared bot path. Scenarios set world spawn to ground level `(0,-60,0)` and
  the harness rejects different origins or bot corrections.
- Slow resource loading can exhaust the startup deadline before any comparison;
  pass `--startup-timeout` as described above. This is a harness
  error, not evidence of event parity or an event mismatch.
