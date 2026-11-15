"""Print the slowest compile units of a Cargo `--timings` report, so CI shows what bounds a build.

    python3 tools/ci/cargo_timings.py target/cargo-timings/cargo-timing.html [--top 25]
"""
import argparse
import json
import re
from pathlib import Path


def units(html: str) -> list[dict]:
    """The report's unit list: Cargo embeds it as `const UNIT_DATA = [...];`."""
    match = re.search(r"const UNIT_DATA = (\[.*?\]);\n", html, re.S)
    if match is None:
        raise ValueError("no UNIT_DATA in the Cargo timings report")
    return json.loads(match.group(1))


def summary(data: list[dict], top: int) -> list[str]:
    end = max(unit["start"] + unit["duration"] for unit in data)
    lines = [f"units={len(data)} wall={end:.1f}s summed={sum(unit['duration'] for unit in data):.1f}s"]
    for unit in sorted(data, key=lambda unit: unit["duration"], reverse=True)[:top]:
        target = unit["target"].strip() or "lib"
        lines.append(f"{unit['duration']:8.1f}s start={unit['start']:7.1f}s {unit['name']} {target} ({unit['mode']})")
    return lines


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("report", type=Path)
    parser.add_argument("--top", type=int, default=25)
    args = parser.parse_args()
    print("\n".join(summary(units(args.report.read_text()), args.top)))
