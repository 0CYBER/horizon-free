#!/usr/bin/env python3
"""Check that real-component raw loader regressions kill their targeted mutants."""
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "crates/horizon-mod-wasm/src/raw.rs"
MUTANTS = [
    ("signature", [("        check_slot(table, &manifest.id, injection)?;", "")]),
    ("wire", [("if entry.hash != shape {", "if false && entry.hash != shape {")]),
    ("wire", [("if entry.hash != shape {", "if entry.signature_hash != injection.hash {")]),
    ("unknown", [(".remove(name.as_str())", ".pop_first().map(|(_, value)| value)")]),
    ("duplicate", [
        (".remove(name.as_str())", ".get(name.as_str()).copied()"),
        ("    if let Some((&id, _)) = expected.first_key_value() {", "    expected.clear();\n    if let Some((&id, _)) = expected.first_key_value() {"),
    ]),
    ("missing", [('exports.func(&interface, "fire")', 'exports.func(&interface, "fallback")')]),
    ("missing-id", [("    if let Some((&id, _)) = expected.first_key_value() {\n        return Err(Refusal::UnfilledPoint { point: id.into() });\n    }\n", "")]),
    ("priority", [(".ordered(listener.priority, name)", "")]),
]


def main():
    original = SOURCE.read_text()
    log_dir = ROOT / "target/raw-mutants"
    log_dir.mkdir(parents=True, exist_ok=True)
    try:
        for index, (case, edits) in enumerate(MUTANTS):
            changed = original
            for before, after in edits:
                assert changed.count(before) == 1, before
                changed = changed.replace(before, after)
            SOURCE.write_text(changed)
            env = dict(os.environ, HORIZON_CONTROLLED_RAW_CASE=case)
            run = subprocess.run(
                ["cargo", "test", "-p", "horizon-mod-wasm", "--lib",
                 "raw::controlled_exports::controlled_load_and_registration", "--", "--nocapture"],
                cwd=ROOT, env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
            )
            (log_dir / f"{index}-{case}.log").write_text(run.stdout)
            assert "could not compile" not in run.stdout, run.stdout
            assert run.returncode != 0, f"survived: {case} ({index})"
            signal = "assertion `left == right` failed" if case == "priority" else f"accepted {case}"
            assert signal in run.stdout, run.stdout
            print(f"killed {index}: {case}", flush=True)
    finally:
        SOURCE.write_text(original)


if __name__ == "__main__":
    main()
