#!/usr/bin/env python3
"""Rename the package crate only; never rename the player's horizon-mods folder.

Declared edits: crate paths, Cargo package/dependency/feature names, whole Rust crate
identifiers (including linker and source-index strings), generated Cargo package
list entries, two crate-tool fixtures, the AGENTS crate map, lockfile ordering,
and rustfmt wrapping. The exact staging regression below is a hand-written addition.
Every other old-name hit is written to --report for review, not changed.
"""
import argparse
from collections import Counter
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parents[1]
OLD, NEW = "horizon-mods", "horizon-mod-package"
OLD_PATH, NEW_PATH = "crates/" + OLD, "crates/" + NEW
OWN_FILES = {"tools/rename_crate.py", "tests/test_rename_crate.py"}
LOCKS = {"Cargo.lock", "launcher/Cargo.lock", "crates/horizon-mod-registry/Cargo.lock"}
PACKAGE_TOOLS = {"tools/pr_ready.py", "tests/test_pr_ready.py"}
STAGING_CHECK = b'''        assert_eq!(
            game["dependencies"]["horizon-mod-package"]["path"].as_str(),
            Some("../horizon-mod-package")
        );
        assert!(!game["dependencies"]
            .as_table()
            .unwrap()
            .contains_key("horizon-mods"));
'''


def run(*args, data=None):
    return subprocess.check_output(args, cwd=ROOT, input=data)


def destination(path):
    return re.sub(r"^" + re.escape(OLD_PATH) + r"(?=/|$)", NEW_PATH, path)


def rewrite(path, source):
    if path in OWN_FILES or path.startswith("crates/horizon-client/assets/horizon/lang/"):
        return source
    source = re.sub(re.escape(OLD_PATH) + r"(?=[^a-zA-Z0-9_-]|$)", NEW_PATH, source)
    source = re.sub(r"\bhorizon_mods\b", "horizon_mod_package", source)
    if Path(path).name == "Cargo.toml":
        source = re.sub(r"(?m)^horizon-mods(?=\s*=)", NEW, source)
        source = re.sub(r"(?m)^(name|package)(\s*=\s*[\"'])horizon-mods([\"'])",
                        lambda m: m[1] + m[2] + NEW + m[3], source)
        source = source.replace("../horizon-mods\"", "../horizon-mod-package\"")
        source = source.replace("../horizon-mods'", "../horizon-mod-package'")
        source = source.replace("dependencies.horizon-mods]", "dependencies.horizon-mod-package]")
    if path in {"crates/cargo-horizon-mod/src/check.rs", "crates/horizon-mod-registry/tests/submit.rs"}:
        source = source.replace("../horizon-mods/tests/", "../horizon-mod-package/tests/")
    if path == "launcher/src/native_mods/api.rs":
        source = re.sub(r'(?m)^(\s*)"horizon-mods",$', rf'\1"{NEW}",', source)
    if path in LOCKS:
        source = source.replace('"horizon-mods"', '"horizon-mod-package"')
    if path in PACKAGE_TOOLS:
        source = source.replace(OLD, NEW)
    if path == "AGENTS.md":
        source = source.replace("`horizon-mods`", "`horizon-mod-package`")
    return source


def normalized(path, data):
    # Git's -I boundary selects text before decoding; binary moved files stay byte-identical.
    source = rewrite(path, data.decode())
    if path.endswith(".rs"):
        edition = "2021" if path.startswith("launcher/") else "2024"
        return run("rustfmt", "--edition", edition, "--config", "skip_children=true",
                   "--emit", "stdout", data=source.encode())
    return source.encode()


def text_paths(base=None):
    args = ["git", "grep", "-Ilz", "-e", OLD, "-e", "horizon_mods"]
    output = run(*args, *([base] if base else []))
    return {name.removeprefix(base + ":") if base else name
            for name in output.decode().split("\0") if name}


def names(base=None):
    args = ["git", "ls-tree", "-rz", "--name-only", base] if base else ["git", "ls-files", "-z"]
    return {name for name in run(*args).decode().split("\0") if name}


def verify(base):
    before, after = names(base), names()
    assert after - OWN_FILES == {destination(path) for path in before - OWN_FILES}, "unexpected file inventory"
    assert not (ROOT / OLD_PATH).exists(), "old crate directory still exists"
    changed = set(run("git", "diff", "--name-only", "--no-renames", base).decode().splitlines())
    text = text_paths(base)
    removed, added = Counter(), Counter()
    files = 0
    for path in sorted(before - OWN_FILES):
        target = destination(path)
        changed_file = path in changed or target in changed
        if not changed_file and path not in text:
            continue
        original = run("git", "show", f"{base}:{path}")
        actual = (ROOT / target).read_bytes()
        expected = normalized(path, original) if path in text else original
        if not changed_file and expected == original:
            continue
        if path == "launcher/src/native_mods/api.rs" and STAGING_CHECK not in original:
            assert actual.count(STAGING_CHECK) == 1, "missing or duplicated staging regression"
            actual = actual.replace(STAGING_CHECK, b"")
        if path in LOCKS:
            assert Counter(expected.splitlines()) == Counter(actual.splitlines()), f"lockfile changed dependencies: {path}"
        else:
            assert actual == expected, f"unexpected content change: {target}"
        removed.update(expected.splitlines())
        added.update(actual.splitlines())
        files += 1
    assert removed == added, "rename-normalized removed/added payload multisets differ"
    print(f"Verified {files} files; {sum(removed.values())} rename-normalized payload lines; removed/added multisets equal")
    print("Declared normalization: crate-only mappings; rustfmt; lockfile ordering; exact 8-line staging regression excluded. Player paths and registry URLs unchanged.")


def report(path):
    rows = []
    for name in sorted(text_paths() - OWN_FILES):
        for line, text in enumerate((ROOT / name).read_text().splitlines(), 1):
            if OLD in text or "horizon_mods" in text:
                rows.append(f"{name}:{line}: {text}")
    path.write_text("Old-name hits retained for review (player paths, registry URLs, prose, test names, and negative checks):\n\n" + "\n".join(rows) + "\n")
    print(f"Retained {len(rows)} old-name lines for review in {path}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--apply", action="store_true")
    mode.add_argument("--verify", metavar="BASE")
    parser.add_argument("--report", type=Path)
    args = parser.parse_args()
    if args.apply:
        move = (ROOT / OLD_PATH).is_dir()
        assert move != (ROOT / NEW_PATH).is_dir(), "expected exactly one crate directory"
        for path in sorted(text_paths() - OWN_FILES):
            original = (ROOT / path).read_bytes()
            updated = normalized(path, original)
            if updated != original:
                (ROOT / path).write_bytes(updated)
        if move:
            run("git", "mv", OLD_PATH, NEW_PATH)
        for manifest in ["Cargo.toml", "launcher/Cargo.toml", "crates/horizon-mod-registry/Cargo.toml"]:
            run("cargo", "metadata", "--offline", "--format-version", "1", "--manifest-path", manifest)
    else:
        verify(args.verify)
    if args.report:
        report(args.report)


if __name__ == "__main__":
    main()
