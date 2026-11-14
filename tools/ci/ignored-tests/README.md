# Ignored-test policy rows

`tools/ci/ignored_tests.py` reads every `*.txt` file here, in file-name order. Each file holds one topic: a
policy (`parity-harness.txt`, `wasm-fixture.txt`, `gpu.txt`, `flaky.txt`, ...) or one tracked failure, named for
its issue (`issue-2177.txt`). A lane that adds a known failure writes its own file; fixing it deletes that file.
A test may be listed in one file only: a second listing fails naming both files.

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
alone in its own process (`wasm-mods.yml` does this for `HORIZON_WASM_FIXTURE`).

`needs-plain` has the same variable and refusal policy as `needs`, but is not an ignored test or a known
failure. The one-process runner selects it with `--exact` without `--ignored`; ordinary data shards skip it
when the variable is absent and require it to pass when present.

`daily` marks a test moved out of `verify` into the daily run (CONTRIBUTING.md "CI tiers"): it is
`#[ignore = "daily"]` in its source, and its row, in `daily-<package>.txt` with the rows sorted, names its cargo target
(`lib`, `bin:NAME` or `test:NAME`) and why it is a daily test (`slow`, `game-boot`, `resources`, `child-process`,
`network`, `wasm-fixture`). It is not a known failure: it runs every day and must pass, and a row whose test no
longer runs fails. Its package's shard runs it when that shard builds its target, and the `daily-rows` leg runs
every other one; verify-static fails a row no leg runs. `python3 tools/ci/move_to_daily.py apply LIST` writes the
attribute and the row (its docstring has the list format). The attribute is the source's mark and the row is the
daily run's list of what must run: an attribute without a row in a package no shard covers would run nowhere.
A launcher test is moved the same way (`daily-horizon-launcher.txt`): its own leg, `daily-rows-launcher`, runs it
from `launcher/`, its own workspace, against its own lockfile. A launcher test that starts the launcher binary also belongs to
the Windows release job `launcher-processes` (`launcher-windows-check.yml`), which runs those `tests/` targets by
`--test` name: add its target there too.
verify-static also checks both directions statically: a `#[ignore = "daily"]` function with no row, or a row with no
such function, fails (`check_ignored_test_rows.py`; matched by package and function name).

`manual` marks an ignored test that never runs in the daily shards, in `manual-<package>.txt`: WHO runs it
(`by-hand`, `subprocess` for a child process its parent test starts, or the workflow file that runs it, such as
`wasm-mods.yml`) and the note says why (a benchmark, a fixture writer, a real GPU, the OS keychain, ...). The shards
skip it.

A rowless `#[ignore]` in a target a package's shard builds (`horizon-client`'s targets, the other shard packages'
libraries) still runs every day with `--include-ignored` and must pass, so it needs no row. Anywhere else it would run
nowhere, so verify-static fails it until it has a row: `daily` to run it in the daily run, or `manual` to say who does.

verify-static fails an un-ignored test that starts a child process (`check_process_tests.py`): it runs a
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
