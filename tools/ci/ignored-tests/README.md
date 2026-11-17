# Ignored-test policy rows

`tools/ci/ignored_tests.py` reads every `*.txt` file here, in file-name order. Each file holds one topic: a
policy (`parity-harness.txt`, `wasm-fixture.txt`, `gpu.txt`, `flaky.txt`, ...) or one tracked failure, named for
its issue (`issue-2177.txt`). A lane that adds a known failure writes its own file; fixing it deletes that file.
A test may be listed in one file only: a second listing fails naming both files.

Horizon keeps its own CI workflows; the fork-only daily, wasm-mods and launcher-windows-check workflows are not
adopted here. This catalogue describes the standalone runners' selections, not automatic CI coverage. Run the
full matrix explicitly with prepared data where available; `by-hand` rows require an explicit local invocation
and their stated inputs. A workflow owner is valid only when that workflow exists in `.github/workflows/`.

One row per line, `#` starts a comment line:

    PACKAGE TEST REFERENCE [flaky|harness|gpu|needs|needs-plain] [# note]
    PACKAGE TEST TARGET daily REASON [# note]
    PACKAGE TEST WHO manual # WHY

REFERENCE is the issue or PR that tracks it (for `needs`, the environment variable). A listed test that passes
fails the job until its row is removed, so a file only shrinks; `flaky` lets one pass. `harness` marks a parity
harness's own step: its harness runs it with the environment it needs, and the job skips it. `gpu` marks a test
that needs a graphics adapter: it runs, and must pass, only where the runner has a Vulkan driver. `needs` marks a
test that reads an input only its harness provides: refused by name when the variable is unset, run and required
to pass when it is set. `ignored_tests.py --needs VARIABLE --one-process-each` runs one variable's tests, each
alone in its own process; supply the built fixtures and `HORIZON_WASM_STD` explicitly for `HORIZON_WASM_FIXTURE`.

`needs-plain` has the same variable and refusal policy as `needs`, but is not an ignored test or a known
failure. The one-process runner selects it with `--exact` without `--ignored`; ordinary data shards skip it
when the variable is absent and require it to pass when present.

`daily` marks a test moved out of `verify` into the daily run (CONTRIBUTING.md "CI tiers"): it is
`#[ignore = "daily"]` in its source, and its row, in a new `daily-PACKAGE-TOPIC.txt` with the rows sorted,
names its cargo target
(`lib`, `bin:NAME` or `test:NAME`) and why it is a daily test (`slow`, `game-boot`, `resources`, `child-process`,
`network`, `wasm-fixture`). It is not a known failure: it must pass when the catalogue runner is invoked, and a row
whose test no longer runs fails. Its package's shard runs it when that shard builds its target, and the `daily-rows`
leg runs every other one; the static guard fails a row no full-matrix leg selects. `python3 tools/ci/move_to_daily.py apply LIST` writes the
attribute and the row (its docstring has the list format). The attribute is the source's mark and the row is the
daily run's list of what must run: an attribute without a row in a package no shard covers would run nowhere.
A launcher test is moved the same way (a new `daily-horizon-launcher-TOPIC.txt`): its own leg, `daily-rows-launcher`,
runs it from `launcher/`, its own workspace, against its own lockfile. Windows-only process tests must be invoked
on Windows; a manual row documents the command when no adopted workflow runs it.
verify-tree also checks both directions statically: a `#[ignore = "daily"]` function with no row, or a row with no
such function, fails (`check_ignored_test_rows.py`). Daily selection matches the package, Cargo target and complete
logical libtest path, including inline modules. Game area folders do not add path segments: the source-only
`horizon-game-discovery` CLI calls the build script's canonical discovery and traverses Rust modules with `syn`.
The guard invokes that small helper once for all targets; it does not build Game, the SDK or prepared resources.

Every descriptive `#[ignore = "..."]` other than `daily` fails unless its exact package, logical test path and
reason appear in `tools/ci/ignored-test-exceptions.json`, with a concrete rationale. These are not prefix
exemptions: a new test or a changed reason needs its own review. Exceptions cover separate parity/wasm fixture
runners, opt-in measurements and fixture writers, interactive/device tests, and intentionally destructive or
private-input checks. Their existing `harness`, `needs`, `gpu` or `manual` rows retain who runs them; these tests
must not be indiscriminately selected as prepared-resource daily tests. Ordinary prepared-resource tests instead
use `daily` and one `resources` or `child-process` row here.

`python3 tools/ci/ignored_inventory.py` emits the machine-readable inventory with canonical identities and ignore
reasons. It accepts `HORIZON_GAME_DISCOVERY=/absolute/path/to/horizon-game-discovery` to reuse a built helper;
otherwise it builds/runs only that lightweight package. Comments, examples in strings and unreferenced source
copies are not treated as runnable tests.

The arismoko/horizon#4451 source inventory found 3,996 actual Rust ignore attributes in 1,139 files, including 3,022 non-daily
descriptive attributes in 853 files (not string/comment examples). Of those, 3,021 have runnable canonical
identities: 2,896 become daily rows (2,872 `resources`, 24 `child-process`), and 125 retain exact documented
exceptions: 77 manual/measurement cases, 22 parity-harness steps, 23 separately supplied inputs, and three
adapter-gated GPU cases. Ordinary `needs HORIZON_TEST_RESOURCES`/`HORIZON_RESOURCES` rows are migrated, not treated
as external-fixture exceptions. The fishing RNG step specifically needs the independent oracle supplied by
`tools/test_fishing_rng_parity.py`, not just prepared resources; every parity assertion stays in that harness step.

One preserved source copy, `crates/horizon-client/src/mobs/living_tests.rs`, is not declared or included by any
Cargo target. Its `living_jump_power_respects_honey_block_and_jump_boost` attribute retains the exact reason
`requires prepared vanilla resources; run explicitly with --ignored`. It has no logical libtest identity while
uncompiled; no prefix exemption is granted. If a target declares it later, its descriptive reason fails the guard
until it is explicitly scheduled. No tests or assertions are removed from this source copy or the migrated tests.

`manual` marks an ignored test that never runs in the daily shards, in `manual-<package>.txt`: WHO runs it
(`by-hand`, `subprocess` for a child process its parent test starts, or an existing workflow file that runs it,
such as `ci.yml`) and the note says why (a benchmark, a fixture writer, a real GPU, the OS keychain, ...). The shards
skip it. Do not name an excluded fork workflow as an owner or substitute a workflow that does not run the test.

A rowless `#[ignore]` in a target a package's shard builds (`horizon-client`'s targets, the other shard packages'
libraries) still runs every day with `--include-ignored` and must pass, so it needs no row. Anywhere else it would run
nowhere, so verify-tree fails it until it has a row: `daily` to run it in the daily run, or `manual` to say who does.

verify-tree fails an un-ignored test that starts a child process (`check_process_tests.py`): it runs a
`Command`, directly or through a helper of its file or one listed in `tools/ci/process-spawns.txt`. Such a test is a
`daily` row with reason `child-process`.

`verify` times its test binaries (`tools/ci/run_test_binaries.py --time-limits 2,5`): a test over 5 s is rerun once,
alone after all binary workers finish, with the same binary and environment. It fails by name if that isolated
rerun also exceeds 5 s, fails, or has no passing timed record; both times are printed. The 2 s warnings stay as they
are, so a warned test stays until someone moves it. Initial times are wall time with three binaries and their test
threads running at once, not isolated measurements. Documentation examples and `verify`'s Python steps are not timed.

Tests that assert process-wide save admission isolate themselves, not the whole shard. For example,
`game::visibility_unwind_tests::visibility_listener_unwind_does_not_block_an_outside_save` uses Game's
`in_own_process` helper to run its callback, unwind and disk-save assertions in an exact-selected child
process. It still runs through the normal parallel shard selection, without a failure-policy row or
special runner selector. Other fixtures' outstanding leases must not be worked around by weakening
the process-wide `passes_settled` save gate.
