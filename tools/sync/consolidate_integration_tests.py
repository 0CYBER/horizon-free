#!/usr/bin/env python3
"""Finish the sync-3 integration-test moves on the marker-merge tree.

Run once from the repository root. Upstream's tests/it destinations are
canonical; equal duplicate rename sources are removed, not copied twice.
"""
from pathlib import Path
import subprocess
import tomllib

UPSTREAM = "7785026e881d185ba7810fd0f80b4a39cf052fd5"


def git(*args):
    return subprocess.check_output(["git", *map(str, args)], text=True)


def main():
    upstream_paths = set(git("ls-tree", "-r", "--name-only", UPSTREAM).splitlines())
    crates = sorted({Path(p).parents[2] for p in upstream_paths
                     if p.endswith("/tests/it/main.rs")})
    crates.append(Path("crates/horizon-game"))  # Owns the fork's executable.
    # The launch contract tests require CARGO_BIN_EXE_horizon-client, owned by horizon-game.
    git("rm", "crates/horizon-client/tests/it/launch_api.rs")
    moved = {}
    lib_tests = {t["name"] for t in tomllib.loads(Path("crates/horizon-client-lib/Cargo.toml").read_text())["test"]}
    for crate in crates:
        tests = crate / "tests"
        dest = tests / "it"
        dest.mkdir(parents=True, exist_ok=True)
        sources = sorted(tests.glob("*.rs")) + sorted((tests / "integration").glob("*.rs"))
        for source in sources:
            # Keep every upstream root-level test in place, including the tests
            # explicitly owned by client-lib, perf's wasm target and player_block_speed.
            if source.parent == tests and str(source) in upstream_paths:
                continue
            if crate.name == "horizon-client" and source.stem in lib_tests:
                continue
            # These fork-only tests each install the same process-wide render slots.
            if crate.name == "horizon-client-render" and source.stem in {"render_slots", "world_view_slots"}:
                continue
            if source.name == "main.rs":
                # Upstream's generated module list replaces the old list and its source-text test.
                git("rm", source)
                continue
            target = dest / source.name
            if target.exists():
                assert source.read_bytes() == target.read_bytes(), (source, target)
                git("rm", source)
            else:
                git("mv", source, target)
            moved[(crate.name, source.stem)] = "it"
        modules = sorted(f.stem for f in dest.glob("*.rs") if f.name != "main.rs")
        for module in modules:
            moved[(crate.name, module)] = "it"
        main_path = dest / "main.rs"
        text = main_path.read_text() if main_path.exists() else "//! Integration tests linked into one binary.\n"
        if crate.name == "horizon-client":
            text = text.replace("mod launch_api;\n", "")
        for name in modules:
            if f"mod {name};" not in text:
                text += f"mod {name};\n"
        main_path.write_text(text)
    for path in Path("tools/ci/ignored-tests").glob("*.txt"):
        lines = []
        for line in path.read_text().splitlines():
            fields = line.split()
            if len(fields) >= 3 and not line.startswith("#"):
                package, test, target = fields[:3]
                if target == "test:integration" and Path("crates", package) in crates:
                    fields[2] = "test:it"
                    line = " ".join(fields)
                elif target.startswith("test:") and (package, target[5:]) in moved:
                    fields[1] = target[5:] + "::" + test
                    fields[2] = "test:it"
                    line = " ".join(fields)
            lines.append(line)
        if lines != path.read_text().splitlines():
            path.write_text("\n".join(sorted(lines) if path.name.startswith("daily-") else lines) + "\n")


if __name__ == "__main__":
    main()
