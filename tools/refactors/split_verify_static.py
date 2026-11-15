#!/usr/bin/env python3
"""Move compile-free verify work into its independently routed required job.

One-shot mechanical migration; hand corrections to setup, gates and tests follow
in separate commits. Step bodies, selection conditions and runner routing stay
byte-for-byte intact in this move.
"""
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github/workflows/dyna-ci.yml"
COMMON = {
    "Require successful check selection",
    "actions/checkout@v4",
    "actions/setup-python@v5",
    "Install Rust toolchain and the runner image's packages",
}
RUNTIME = {
    "Check Dyna dependencies (install fallback without root)",
    "Swatinem/rust-cache@v2",
    "Build the runtime holder test actor",
    "Test Rust core and voxel kernel correctness",
    "Build the source stage generator (test profile)",
    "Validate prebuilt source stage and listed slots",
    "Test Rust documentation examples",
    "Report test compiler cache",
}
STATIC = {
    "Fetch this commit's history",
    "Keep game artifacts out of Git",
    "Check duplicate public native functions",
    "Built-in addresses stay in their home",
    "Crates follow the layers",
    "Test offline research tooling",
    "Build the mod-host map tables",
    "Upload the mod-host map tables",
    "Report optional mod-host map upload failure",
}


def main() -> None:
    source = WORKFLOW.read_text()
    if "\n  verify-static:\n" in source:
        raise SystemExit("verify-static already exists; this migration is one-shot")
    start = source.index("  verify:\n")
    end = source.index("  # The #[ignore]d library tests", start)
    header, body = source[start:end].split("    steps:\n", 1)
    lines = body.splitlines(keepends=True)
    positions = []
    for index, line in enumerate(lines):
        match = re.fullmatch(r"      - (?:name|uses): (.+)\n", line)
        if match is None:
            continue
        beginning = index
        while beginning and (lines[beginning - 1].startswith("      #")
                             or not lines[beginning - 1].strip()):
            beginning -= 1
        positions.append((beginning, match[1]))
    names = [name for _, name in positions]
    expected = COMMON | RUNTIME | STATIC
    if len(names) != len(set(names)) or set(names) != expected:
        raise SystemExit(f"step assignment changed: missing={expected - set(names)}, "
                         f"unknown={set(names) - expected}, duplicate={len(names) != len(set(names))}")
    if positions[0][0] != 0:
        raise SystemExit("unexpected prelude before the selection failure guard")
    runtime = []
    static = []
    for number, (beginning, name) in enumerate(positions):
        stop = positions[number + 1][0] if number + 1 < len(positions) else len(lines)
        block = "".join(lines[beginning:stop])
        if name in COMMON | RUNTIME:
            runtime.append(block)
        if name in COMMON | STATIC:
            static.append(block)
    runtime_job = header + "    steps:\n" + "".join(runtime)
    static_header = header.replace("  verify:\n", "  verify-static:\n", 1)
    static_job = static_header + "    steps:\n" + "".join(static)
    WORKFLOW.write_text(source[:start] + runtime_job + "\n" + static_job + source[end:])


if __name__ == "__main__":
    main()
