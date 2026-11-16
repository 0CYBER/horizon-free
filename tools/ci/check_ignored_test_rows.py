#!/usr/bin/env python3
"""verify-static's guard for tools/ci/ignored-tests/: read every policy file as the ignored-test readers do.

Fails on a malformed row, a test listed in two files, a folder with no rows or with a file other than `*.txt`
and README.md, and a file at the retired single-list path. The shared reader validates both `needs`
and `needs-plain` fixture rows; neither is a known failure. Every `daily` row must name a package of a workspace (the game's or the launcher's) and
one of its test targets, and some leg of the daily run's matrix must run it: its package's shard, or the `daily-rows`
leg (tools/ci/ignored_shards.py), so no moved test runs nowhere. Builds only the source-inventory helper,
not the game or SDK.

Every `#[ignore]`d test that no daily shard runs (rowless_problems) has a row of some mark: `daily`, a known
failure, `needs`, `manual`, ...; otherwise it would run nowhere unseen. In a shard's own targets a rowless
`#[ignore]` is fine: the shard runs it with `--include-ignored`, and it must pass.

In addition to the legacy physical-source checks below, the canonical inventory checks package, target and
full logical test path.

The legacy rows and physical-source checks agree both ways (read with tools/ci/rust_tests.py): every function marked
`#[ignore = "daily"]` in a workspace package's sources has a daily row of that package whose test path ends in its
name, and every daily row names a function of that name marked so in its package's sources. A source file compiled
into two targets (horizon-client-render's gfx/sunrise_fan.rs, also built by its `sunrise_shader` test) has one
attribute and a row per target; both rows find it. The limit: rows carry no file and the source has no module path
(a `#[path]` or a game area's derived module can place a file anywhere), so the match is by package and function
name. Two marked functions of one name in one package are satisfied by one row naming either; the daily run, which
runs each row by its exact path, still fails a row whose test does not run.
"""
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from tools.ci import ignored_inventory, ignored_shards, ignored_tests, rust_tests

ROOT = Path(__file__).resolve().parents[2]
ADD_ROW = ("add its row: `python3 tools/ci/move_to_daily.py apply LIST` (its docstring has the list format), or write "
           "`PACKAGE TEST TARGET daily REASON` into tools/ci/ignored-tests/daily-PACKAGE-TOPIC.txt")


def daily_problems(rows, targets, matrix):
    """Why each daily row would not run in the daily matrix `matrix` (shard names), given the workspace's targets."""
    problems = []
    groups = ignored_tests.daily_rows_groups(rows)
    for row in rows:
        where = f"{row.file}: {row.package} {row.test}"
        if row.package not in targets:
            problems.append(f"{where}: {row.package} is not a workspace package")
        elif row.reference not in targets[row.package]:
            problems.append(f"{where}: {row.package} has no target {row.reference}")
        elif ignored_tests.shard_runs(row.package, row.reference):
            if not any(ignored_tests.parse_shard(shard)[0] == row.package for shard in matrix):
                problems.append(f"{where}: no leg runs {row.package}'s shard")
        elif (leg := ignored_shards.daily_rows_leg(row.package)) not in matrix or \
                row.test not in groups.get((row.package, row.reference), ()):
            problems.append(f"{where}: no leg runs it (the {leg} leg is missing)")
    return problems


def source_packages():
    """Each member's source folder, in every workspace (the launcher's too) -> the packages whose sources it holds
    (the deepest folder wins, and crates/horizon-client also builds horizon-client-lib:
    tools/ci/ignored_shards.py SHARED_SOURCES)."""
    return {member: set(ignored_shards.SHARED_SOURCES.get(Path(member).name, (name,))) | {name}
            for name, (member, _) in ignored_shards.workspace_members().items()}


def target_kind(member, path):
    """The kind of target a member's source file builds into, as a daily row's TARGET begins: `test` (tests/),
    `bin` (src/main.rs, src/bin/), `example` or `bench`, else `lib`."""
    inside = path[len(member) + 1:]
    for prefix, kind in (("tests/", "test"), ("src/bin/", "bin"), ("examples/", "example"), ("benches/", "bench")):
        if inside.startswith(prefix):
            return kind
    return "bin" if inside == "src/main.rs" else "lib"


def ignored_functions(root=ROOT):
    """(path, line, function name, packages, target kind, daily) of every unconditionally `#[ignore]`d test
    function in the tracked sources of every workspace."""
    owners = source_packages()
    listing = subprocess.run(["git", "ls-files", "-z", "--", "*.rs"], cwd=root, capture_output=True, text=True,
                             check=True)
    found = []
    for path in filter(None, listing.stdout.split("\0")):
        folder = max((member for member in owners if path.startswith(member + "/")), key=len, default=None)
        if folder is None:
            continue
        source = (root / path).read_text(errors="replace")
        if "ignore" not in source:
            continue
        for function in rust_tests.functions(source, rust_tests.DAILY_KEEP):
            if function.is_test and function.is_ignored:
                found.append((path, function.line, function.name, owners[folder], target_kind(folder, path),
                              function.is_daily))
    return found


def daily_functions(root=ROOT):
    """(path, line, function name, packages) of every function marked `#[ignore = "daily"]` in tracked sources."""
    return [(path, line, name, packages) for path, line, name, packages, _, daily in ignored_functions(root) if daily]


def rowless_problems(rows, functions):
    """Ignored tests no daily shard runs and no row of any mark names: they run nowhere, and nothing says so.

    A package's shard runs every test of the targets it builds with `--include-ignored`, so there a rowless
    `#[ignore]` still runs daily and must pass; anywhere else (another package, or a shard package's target its shard
    does not build) it needs a row. Matched by package and function name."""
    named = {(row.package, row.test.rsplit("::", 1)[-1]) for row in rows}
    problems = []
    for path, line, name, packages, kind, _ in functions:
        if any(ignored_tests.shard_runs(package, "lib" if kind == "lib" else f"{kind}:") for package in packages):
            continue
        if not any((package, name) in named for package in packages):
            problems.append(f"{path}:{line}: `{name}` is #[ignore]d in {' or '.join(sorted(packages))}, which no daily "
                            "shard runs, and no row names it, so it runs nowhere: mark it #[ignore = \"daily\"] with its "
                            f"row to run it daily ({ADD_ROW}), or write a `manual` row in "
                            "tools/ci/ignored-tests/manual-PACKAGE.txt saying who runs it and why")
    return problems


def source_problems(rows, functions):
    """Marked functions with no row, and rows with no marked function (matched by package and function name)."""
    problems = []
    named = {(row.package, row.test.rsplit("::", 1)[-1]) for row in rows}
    for path, line, name, packages in functions:
        if not any((package, name) in named for package in packages):
            problems.append(f"{path}:{line}: `{name}` is #[ignore = \"daily\"] but no daily row of "
                            f"{' or '.join(sorted(packages))} names it, so the daily run may never run it: {ADD_ROW}")
    marked = {(package, name) for _, _, name, packages in functions for package in packages}
    for row in rows:
        if (row.package, row.test.rsplit("::", 1)[-1]) not in marked:
            problems.append(f"{row.file}: {row.package} {row.test}: no function `{row.test.rsplit('::', 1)[-1]}` "
                            f"in {row.package}'s sources is #[ignore = \"daily\"]: restore the attribute "
                            "(tools/ci/move_to_daily.py apply) or remove the row")
    return problems


if __name__ == "__main__":
    rows = ignored_tests.listed_entries()
    print(f"{len(rows)} rows in {len({row.file for row in rows})} files of {ignored_tests.POLICY_PATH}/")
    daily = [row for row in rows if row.mark == "daily"]
    problems = daily_problems(daily, ignored_shards.workspace_targets(),
                              [*ignored_shards.expand(ignored_shards.SHARDS), *ignored_shards.DAILY_ROWS_LEGS.values()])
    ignored = ignored_functions()
    functions = [(path, line, name, packages) for path, line, name, packages, _, daily in ignored if daily]
    problems += source_problems(daily, functions)
    problems += rowless_problems(rows, ignored)
    problems += ignored_inventory.problems(rows, ignored_inventory.tests(), ignored_inventory.exceptions())
    for problem in problems:
        print(f"::error::{problem}")
    print(f"{len(daily)} daily rows, {len(functions)} #[ignore = \"daily\"] functions, {len(ignored)} ignored tests; "
          f"{len(problems)} problems")
    sys.exit(1 if problems else 0)
