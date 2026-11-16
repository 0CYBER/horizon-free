#!/usr/bin/env python3
"""verify-static's guard of the unit tier: a test that starts a child process is a daily test.

Fails naming every `#[test]` function that is not `#[ignore]`d and starts a process, directly or through a helper
function of the same file. A function starts one when its body:
- runs a built `Command`: `.spawn()` (a thread's `spawn` takes its closure, so an empty one is a process's), or
  `.output()` / `.status()` in a file that names `Command` and a body that builds one (`Command`, `.arg(`, `.args(`,
  `.env(`, `.current_dir(`), as other types have methods of those two names (a status effect's `.status()`);
- calls a helper listed in tools/ci/process-spawns.txt (one file: a helper in another file or package that starts a
  process; a `use ... as` alias of one counts too);
- calls a function of the same file that starts one (bare, `self.`, `Self::`, `super::` or `Type::`), until nothing
  changes.
Starting a process on a loaded machine can take ten times its measured time (cargo-horizon-mod's internals test:
0.5 s measured, 5.47 s on the runner), so the daily run, not `verify`, runs these tests. Move one with
`#[ignore = "daily"]` and its row, `python3 tools/ci/move_to_daily.py apply LIST`, reason `child-process`.

Limits (tools/ci/rust_tests.py reads the source, it does not compile it): calls are matched by name, so a
same-file helper `new` that starts a process also matches another type's `new(` in that file; a helper in another
file is followed only through the list, and a call through a trait object, a stored closure or a macro is not
followed. A Windows-only test (`#[cfg(windows)]`) is not checked: `verify` runs on Linux, the Windows jobs run it. The
launcher (its own workspace) is not read: its process-starting `tests/` targets run in its Windows jobs. Builds
nothing.

Usage: check_process_tests.py [--list]   (--list: tab-separated `path line test packages how` of every offender)
"""
import re
import subprocess
import sys
import tomllib
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from tools.ci import check_game_boot_tests, rust_tests  # noqa: E402

ROOT = Path(__file__).resolve().parents[2]
SPAWNS = Path(__file__).with_name("process-spawns.txt")
SPAWN = re.compile(r"\.\s*spawn\s*\(\s*\)")
FINISH = re.compile(r"\.\s*(?:output|status)\s*\(\s*\)")
BUILDS = re.compile(r"\bCommand\b|\.\s*(?:args?|env|current_dir)\s*\(")
# Kept unblanked by rust_tests.functions(keep=...), so `#[cfg(target_os = "windows")]` is seen.
WINDOWS = frozenset({"windows"})
WINDOWS_ONLY = re.compile(r"#\s*\[\s*cfg\s*\(\s*(?:windows|target_os\s*=\s*\"windows\")\s*\)\s*\]")
# The crates/horizon-client tree also builds horizon-client-lib (its [lib] path reaches into it).
SHARED = {"crates/horizon-client": ("horizon-client", "horizon-client-lib")}


def spawn_names(path=SPAWNS):
    """The listed process-starting helpers (`Type::function`, `module::function` or `function`); `#` starts a
    comment."""
    names = []
    for number, line in enumerate(path.read_text().splitlines(), 1):
        entry = line.split("#", 1)[0].strip()
        if entry and not re.fullmatch(r"(?:[A-Za-z_]\w*::)*[A-Za-z_]\w*", entry):
            sys.exit(f"{path.name}:{number}: expected a function name or `Type::function`, found {entry!r}")
        names += [entry] if entry else []
    return names


def local_call(name):
    """A call of the same file's function `name`: bare, as a method of `self`, or through `Self::`, `super::` or a
    type (`Scene::new(`); never through a module path (`thread::spawn(` is not a helper `spawn`)."""
    return re.compile(r"(?<![\w$:.])(?:(?:Self|super|[A-Z]\w*)\s*::\s*|self\s*\.\s*)?" + re.escape(name)
                      + r"\s*(?:::\s*<[^(){};]*>\s*)?\(")


def offenders_in(source, listed):
    """(test function, how it starts a process) for every un-ignored test in `source` that starts one."""
    # A listed helper imported under another name (`use crate::leases::isolated_test as isolated;`) is listed too.
    names = [*listed, *(alias for name in listed
                        for alias in re.findall(rf"\b{re.escape(name.rsplit('::', 1)[-1])}\s+as\s+(\w+)", source))]
    command = "Command" in source
    if not SPAWN.search(source) and not command and not any(
            all(part in source for part in name.split("::")) for name in names):
        return []
    patterns = {name: check_game_boot_tests.call(name) for name in names}
    found = rust_tests.functions(source, WINDOWS)
    starting = {}

    def how(function):
        if SPAWN.search(function.body) or command and FINISH.search(function.body) and BUILDS.search(function.body):
            return "runs a Command"
        return next((name for name, pattern in patterns.items() if pattern.search(function.body)), None) or next(
            (f"{name} -> {chain}" for name, chain in starting.items() if local_call(name).search(function.body)), None)

    changed = True
    while changed:
        changed = False
        for function in found:
            if not function.is_test and function.name not in starting and (chain := how(function)):
                starting[function.name] = chain
                changed = True
    return [(function, chain) for function in found
            if function.is_test and not function.is_ignored and not WINDOWS_ONLY.search(function.attribute_text)
            and (chain := how(function))]


def offenders(root=ROOT):
    """(path, function, how, packages) for every un-ignored test of the game workspace that starts a process."""
    listed = spawn_names()
    folders = {}
    for member in tomllib.loads((root / "Cargo.toml").read_text())["workspace"]["members"]:
        folders[member] = SHARED.get(member, (tomllib.loads((root / member / "Cargo.toml").read_text())["package"]["name"],))
    tracked = subprocess.run(["git", "ls-files", "-z", "--", "*.rs"], cwd=root, capture_output=True,
                             text=True, check=True).stdout.split("\0")
    found = []
    for path in sorted(filter(None, tracked)):
        folder = max((member for member in folders if path.startswith(member + "/")), key=len, default=None)
        if folder is not None:
            for function, chain in offenders_in((root / path).read_text(errors="replace"), listed):
                found.append((path, function, chain, folders[folder]))
    return found


def main(arguments):
    found = offenders()
    if arguments == ["--list"]:
        for path, function, chain, packages in found:
            print(f"{path}\t{function.line}\t{function.name}\t{' '.join(packages)}\t{chain}")
        return 0
    for path, function, chain, _ in found:
        print(f"::error::{path}:{function.line}: test `{function.name}` starts a process ({chain}): a test that "
              "starts a process runs daily; mark it #[ignore = \"daily\"] and add its row, reason child-process "
              "(tools/ci/move_to_daily.py, tools/ci/ignored-tests/README.md)")
    print(f"{len(found)} unignored tests start a process ({len(spawn_names())} helpers in tools/ci/{SPAWNS.name})")
    return 1 if found else 0


if __name__ == "__main__":
    if any(argument != "--list" for argument in sys.argv[1:]):
        sys.exit(__doc__)
    sys.exit(main(sys.argv[1:]))
