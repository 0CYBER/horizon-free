#!/usr/bin/env python3
"""Mechanical EntityTypeKey migration for #3549, ruling 2026-11-13 04:49.

Run --apply, commit this script, edits.json and ONLY its reported output paths,
then implement semantic provider/key/Wire fixes in separate commits. This pass
intentionally does not make String-producing provider bodies compile against the
new key return type. Existing name consumers explicitly request key.id(host);
key caching and comparison changes belong to the later hand fixes.

The earlier equip-decision foundation is unrelated pre-existing working-tree
work and is not part of this script's output. No cargo/generator invocation is
performed here. rustfmt is part of the reproducible mechanical output.
"""
import argparse
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[3]
RULES = Path(__file__).with_name("edits.json")


def compact(text):
    # Used only to recognize already-applied edits after rustfmt (including argument commas). Writes always
    # use the exact captured source, never a whitespace-normalized replacement.
    return "".join(text.split()).replace(",)", ")")


def plan(root, rules):
    changes = {}
    counts = {}
    for edit in rules["edits"]:
        path = root / edit["path"]
        text = changes.get(path, path.read_text())
        old, new, count = edit["old"], edit["new"], edit["count"]
        if compact(text).count(compact(new)) == count:
            continue
        actual = text.count(old)
        if actual != count:
            raise SystemExit(f"{edit['path']} ({edit['label']}): expected {count} old occurrences, found {actual}; stop and inspect source drift")
        changes[path] = text.replace(old, new)
        counts[edit["label"]] = counts.get(edit["label"], 0) + count
    return changes, counts


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=ROOT)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--apply", action="store_true")
    mode.add_argument("--dry-run", action="store_true")
    mode.add_argument("--check", action="store_true", help="require an already-migrated tree; do not write")
    parser.add_argument("--report", type=Path, help="optional local JSON report, not source output")
    parser.add_argument("--only", action="append", help="limit migration to these paths after a merge")
    args = parser.parse_args()
    rules = json.loads(RULES.read_text())
    if args.only:
        rules["edits"] = [edit for edit in rules["edits"] if edit["path"] in args.only]
    changes, counts = plan(args.root, rules)
    paths = sorted(changes)
    if args.apply:
        # Preflight every edit above before writing any file.
        for path in paths:
            path.write_text(changes[path])
        rust = [str(path) for path in paths if path.suffix == ".rs"]
        if rust:
            subprocess.run(["rustfmt", "--edition", "2024", "--config", "skip_children=true", *rust], check=True)
        remaining, _ = plan(args.root, rules)
        if remaining:
            raise SystemExit("migration was not idempotent after formatting")
    report = {
        "phase": "mechanical migration only; stop before semantic hand fixes",
        "paths": [str(path.relative_to(args.root)) for path in paths],
        "replacements": counts,
        "hand_fixes": rules["hand_fixes"],
        "other_reads": rules["other_reads"],
    }
    print(json.dumps(report, indent=2))
    if args.report:
        args.report.write_text(json.dumps(report, indent=2) + "\n")
    if args.check and paths:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
