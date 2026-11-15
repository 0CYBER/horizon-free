#!/usr/bin/env python3
"""Kill wrong-owner dispatch faults through the real Game tick, restoring source.

Needs HORIZON_TEST_RESOURCES and isolated game home environment variables.
"""
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / "crates/horizon-client/src/game/world/ordered_tick.rs"
original = SOURCE.read_text()
cmd = ["cargo", "test", "-p", "horizon-client", "--lib",
       "dispatch_ticks_both_owners_of_each_located_kind", "--", "--ignored"]
try:
    baseline = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True)
    assert baseline.returncode == 0 and "1 passed" in baseline.stdout, baseline.stdout + baseline.stderr
    for kind in ["tnts", "falling", "orbs"]:
        old = f"let Some(i) = self.{kind}.locate(join) else {{\n                        continue;\n                    }};"
        assert original.count(old) == 1, kind
        SOURCE.write_text(original.replace(old, old + "\n                    let i = 0;"))
        result = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True)
        output = result.stdout + result.stderr
        assert result.returncode != 0 and "test result: FAILED" in output, output
        print(f"{kind}: force slot zero: killed", flush=True)
finally:
    SOURCE.write_text(original)
