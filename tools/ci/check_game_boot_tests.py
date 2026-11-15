#!/usr/bin/env python3
"""verify-static's guard of the unit tier: a test that boots the game is a daily test.

Fails naming every `#[test]` function that is not `#[ignore]`d and calls a game-boot fixture, directly or through a
helper function of the same file that does. The fixtures are the names in tools/ci/game-boot-fixtures.txt (one
file: add a new fixture there). `verify` runs without prepared data, so such a test either fails there or returns
early having checked nothing; the daily run (tools/ci/ignored-tests/) runs it with the data. Move one with
`#[ignore = "daily"]` and its row, `python3 tools/ci/move_to_daily.py apply LIST`.

Limits (tools/ci/rust_tests.py reads the source, it does not compile it): calls are matched by name, so a helper
in another file, a method reached through a trait, or a call a macro writes is not followed; a same-file helper
and a fixture match any function or method of that name. Builds nothing.

Usage: check_game_boot_tests.py [FILE.rs...]   (default: every tracked .rs file under crates/ and launcher/)
"""

import re
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from tools.ci import rust_tests  # noqa: E402

ROOT = Path(__file__).resolve().parents[2]
FIXTURES = Path(__file__).with_name("game-boot-fixtures.txt")


def fixtures(path=FIXTURES):
    """The fixture names (`with_game`, `Game::new`), from the list's rows; `#` starts a comment."""
    names = []
    for number, line in enumerate(path.read_text().splitlines(), 1):
        entry = line.split("#", 1)[0].strip()
        if not entry:
            continue
        if not re.fullmatch(r"(?:[A-Za-z_]\w*::)*[A-Za-z_]\w*", entry):
            sys.exit(f"{path.name}:{number}: expected a function name or `Type::function`, found {entry!r}")
        names.append(entry)
    if not names:
        sys.exit(f"{path.name} lists no fixtures")
    return names


def call(name):
    """A call of `name` (a function, a method or `Type::function`), with or without a turbofish."""
    return re.compile(r"(?<![\w$])" + r"\s*::\s*".join(map(re.escape, name.split("::")))
                      + r"\s*(?:::\s*<[^(){};]*>\s*)?\(")


def offenders(source, names):
    """(test function, what it reaches) for every unignored test in `source` that reaches a fixture in `names`."""
    calls = {name: call(name) for name in names}
    # Helpers are followed within the file only, so a file that calls no fixture has no offender.
    if not any(all(part in source for part in name.split("::")) and calls[name].search(source) for name in names):
        return []
    found = rust_tests.functions(source)
    # Same-file helpers that reach a fixture, until nothing changes: name -> the chain that boots.
    booting = {}
    changed = True
    while changed:
        changed = False
        for function in found:
            if function.is_test or function.name in booting:
                continue
            for name, pattern in list(calls.items()):
                if pattern.search(function.body):
                    booting[function.name] = name if name in names else f"{name} -> {booting[name]}"
                    calls[function.name] = call(function.name)
                    changed = True
                    break
    result = []
    for function in found:
        if function.is_test and not function.is_ignored:
            for name, pattern in calls.items():
                if pattern.search(function.body):
                    result.append((function, name if name in names else f"{name} -> {booting[name]}"))
                    break
    return result


def tracked_sources():
    listing = subprocess.run(["git", "ls-files", "-z", "--", "crates/*.rs", "launcher/*.rs"], cwd=ROOT, capture_output=True,
                             text=True, check=True)
    return [path for path in listing.stdout.split("\0") if path]


def main(paths):
    names = fixtures()
    problems = []
    for path in paths or tracked_sources():
        for function, chain in offenders((ROOT / path).read_text(errors="replace"), names):
            problems.append(f"{path}:{function.line}: test `{function.name}` boots the game ({chain})")
    for problem in problems:
        print(f"::error::{problem}: a game-boot test runs daily; mark it #[ignore = \"daily\"] and add its row "
              "(tools/ci/move_to_daily.py, tools/ci/ignored-tests/README.md)")
    print(f"{len(problems)} unignored tests boot the game ({len(names)} fixtures in "
          f"tools/ci/{FIXTURES.name})")
    return 1 if problems else 0


if __name__ == "__main__":
    if any(argument.startswith("-") for argument in sys.argv[1:]):
        sys.exit(__doc__)
    sys.exit(main(sys.argv[1:]))
