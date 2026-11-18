"""Source-only ignored-test inventory, using horizon-game-discovery's canonical module traversal.

The small Rust tool parses source with syn and calls the same game-area discovery as the build script.
It never builds the game or loads prepared data. One invocation covers every workspace and test target.
"""
import json
import os
import subprocess
import sys
import tomllib
from functools import lru_cache
from pathlib import Path
from typing import NamedTuple

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from tools.ci import ignored_shards

ROOT = Path(__file__).resolve().parents[2]

EXCEPTIONS = Path(__file__).with_name("ignored-test-exceptions.json")


def exceptions(path=EXCEPTIONS):
    """Exact identities/reasons with separately documented runners; never a reason-prefix exemption."""
    found = {}
    for entry in json.loads(path.read_text()):
        key = (entry["package"], entry["target"], entry["test"], entry["reason"])
        if key in found or not entry["rationale"].strip():
            raise ValueError(f"{path}: duplicate or unexplained exception: {key}")
        found[key] = entry
    return found


def problems(rows, functions, allowed):
    """A descriptive ignore must have an exact exception; daily selection uses the complete libtest identity.

    Every daily row is validated: rows sharing an identity are kept together instead of one silently
    overwriting the other, and a duplicate identity across policy files is itself reported.
    """
    found = []
    # Keep *all* daily rows, keyed by identity. A dict comprehension keyed by identity would let a later
    # row from another policy file erase an earlier one, leaving that row's file never validated.
    daily = {}
    for row in rows:
        if row.mark == "daily":
            daily.setdefault((row.package, row.reference, row.test), []).append(row)
    marked = set()
    for function in functions:
        if function.reason is None:
            continue
        identity = (function.package, function.target, function.name)
        location = f"{function.path}:{function.line}: {function.package} {function.name}"
        if function.reason == "daily":
            marked.add(identity)
            if identity not in daily:
                found.append(f'{location}: #[ignore = "daily"] has no exact daily row for {function.target}')
        elif (*identity, function.reason) not in allowed:
            found.append(f"{location}: descriptive ignore {function.reason!r} is not daily or an explicitly "
                         "documented exception; migrate it with its exact logical identity")
    for identity, group in daily.items():
        if len(group) > 1:
            files = ", ".join(row.file for row in group)
            found.append(f"{files}: {' '.join(identity)}: duplicate daily rows select the same test identity")
        for row in group:
            if identity not in marked:
                found.append(f"{row.file}: {' '.join(identity)}: daily row selects no #[ignore = \"daily\"] test")
    return found

class Test(NamedTuple):
    package: str
    target: str
    path: str
    line: int
    name: str
    reason: str | None  # None for a bare ignore or an unignored test; empty string is a descriptive ignore.
    ignored: bool


def target_roots(root=ROOT):
    """Cargo target roots, including explicit paths and the launcher's separate workspace."""
    roots = set()
    for workspace in ignored_shards.WORKSPACES:
        table = tomllib.loads((root / workspace / "Cargo.toml").read_text())
        members = ([workspace] if "package" in table else []) + [
            Path(workspace, member).as_posix() for member in table["workspace"].get("members", ())]
        for member in members:
            folder = root / member
            manifest = tomllib.loads((folder / "Cargo.toml").read_text())
            package = manifest["package"]
            name = package["name"]
            if "lib" in manifest or (folder / "src/lib.rs").is_file():
                roots.add((name, "lib", (folder / manifest.get("lib", {}).get("path", "src/lib.rs")).resolve(),
                           folder.resolve()))
            for kind, plural, default in (("bin", "bins", "src/bin"), ("test", "tests", "tests"),
                                           ("example", "examples", "examples")):
                explicit = {entry["name"]: entry for entry in manifest.get(kind, ())}
                candidates = {}
                if package.get(f"auto{plural}", True):
                    candidates.update({path.stem: path for path in (folder / default).glob("*.rs")})
                    candidates.update({path.parent.name: path for path in (folder / default).glob("*/main.rs")})
                    if kind == "bin" and (folder / "src/main.rs").is_file():
                        candidates[name] = folder / "src/main.rs"
                for target, entry in explicit.items():
                    if "path" in entry:
                        candidates[target] = folder / entry["path"]
                    elif target not in candidates:
                        candidates[target] = folder / default / f"{target}.rs"
                roots.update((name, f"{kind}:{target}", path.resolve(), folder.resolve())
                             for target, path in candidates.items())
    return sorted((package, target, path.relative_to(root.resolve()).as_posix(),
                   folder.relative_to(root.resolve()).as_posix()) for package, target, path, folder in roots)


@lru_cache(maxsize=None)
def tests(root=ROOT):
    """Canonical full identities and original ignore reasons, in one batched source-only invocation."""
    root = Path(root).resolve()
    binary = os.environ.get("HORIZON_GAME_DISCOVERY")
    command = [binary] if binary else ["cargo", "run", "--quiet", "--locked", "-p", "horizon-game-discovery",
                                      "--bin", "horizon-game-discovery", "--"]
    inputs = "".join("\t".join(row) + "\n" for row in target_roots(root))
    result = subprocess.run([*command, str(root)], input=inputs, capture_output=True, text=True, cwd=ROOT)
    if result.returncode:
        raise SystemExit(f"canonical Rust test inventory failed ({result.returncode}):\n{result.stderr}")
    found = []
    for line in result.stdout.splitlines():
        found.append(Test(**json.loads(line)))
    return tuple(found)


if __name__ == "__main__":
    ignored = [test._asdict() for test in tests() if test.ignored]
    json.dump(ignored, sys.stdout, indent=2)
    print()
