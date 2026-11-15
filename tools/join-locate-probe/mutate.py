#!/usr/bin/env python3
"""Kill production lookup, store-back and checked-update faults, restoring source."""
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / "crates/horizon-client/src/join_seq.rs"
original = SOURCE.read_text()
# Exact source anchors are unique; no test edits are made.
faults = [
    ("duplicate selection", "Ok(i) => Some(i),",
     "Ok(_) => self.0.iter().position(|entity| entity.join_seq() == join),"),
    ("store_back direct candidate", ".filter(|&i| self.0.get(i).is_some_and(|old| old.join_seq() == join))",
     ".filter(|_| false)"),
    ("locate hit", "Ok(i) => Some(i),", "Ok(i) => Some(i + 1),"),
    ("locate fallback", "Err(_) => self.0.iter().position(|entity| entity.join_seq() == join)",
     "Err(_) => None"),
    ("locate key", "entity.join_seq() == join", "entity.join_seq() != join"),
    ("store_back join", "let join = entity.join_seq();", "let join = JoinSeq::next();"),
    ("store_back stale slot", ".filter(|&i| self.0.get(i).is_some_and(|old| old.join_seq() == join))",
     ".filter(|&i| self.0.get(i).is_some())"),
    ("store_back relocation", ".or_else(|| self.locate(join))", ".or_else(|| None)"),
    ("store_back absent", "return false;", "return true;"),
    ("store_back lifetime", "std::mem::swap(old.attachment_lifetime(), new.attachment_lifetime());", ""),
    ("update registration", "before.is_none_or(|old| entity", "true || before.is_none_or(|old| entity"),
]
cmd = ["cargo", "test", "--manifest-path", str(Path(__file__).with_name("Cargo.toml")),
       "--lib", "join_seq::tests::"]
try:
    subprocess.run(cmd, check=True, cwd=ROOT, capture_output=True)
    for name, old, new in faults:
        assert original.count(old) == 1, (name, old)
        SOURCE.write_text(original.replace(old, new))
        result = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True)
        output = result.stdout + result.stderr
        assert result.returncode != 0 and "test result: FAILED" in output, output
        print(f"{name}: killed", flush=True)
finally:
    SOURCE.write_text(original)
