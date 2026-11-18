#!/usr/bin/env python3
"""Pins the SHA-256 of every release published before signing began (docs/security.md, "Signed updates").

    python3 tools/ci/legacy_releases.py           # rewrite crates/horizon-release-sign/legacy-releases.txt
    python3 tools/ci/legacy_releases.py --check   # exit 1 if the file is out of date

Releases from before signing carry no SHA256SUMS.sig, so the launcher and `horizon-server update` can't check them
against Horizon's key. Their archives' SHA-256s, as GitHub computed them at upload, are built into the binaries
instead, so a rollback to one stays possible and nobody can swap its files later. Every release with a
SHA256SUMS.sig is left out: its signature is what counts. Run it once, just before the launcher starts refusing
unsigned releases, when every release since the first signed one is signed; after that the file never changes.
"""

import argparse
import json
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "crates" / "horizon-release-sign" / "legacy-releases.txt"
REPO = "horizon-rust/horizon"
HEADER = """\
# Releases published before signing began, with their archives' SHA-256 as GitHub computed them at upload:
# `<tag> <asset> <sha256>` per line, written by tools/ci/legacy_releases.py. The launcher and horizon-server
# install these releases only with exactly these files (docs/security.md, "Signed updates"); every later release
# is checked against its signed SHA256SUMS instead. Changing this file needs Ludovic's review (.github/CODEOWNERS).
"""


def tag_key(tag):
    # The numeric version is the leading run of dot-separated numbers; anything after it (e.g. "-rc.1")
    # is a pre-release label and must sort before the final release of that same version. Folding the
    # label's digits into the version made v1.2.3-rc.1 compare as a larger version than v1.2.3.
    match = re.match(r"^(\D*)(\d+(?:\.\d+)*)(.*)$", tag)
    if not match:
        return (0 if tag.startswith("v") else 1, (), 1, ())
    prefix = 0 if match.group(1).startswith("v") else 1
    base = tuple(int(part) for part in match.group(2).split("."))
    label = match.group(3).lstrip("-").lower()
    # Keep candidate numbers numeric too: rc.2 precedes rc.10, not the other way around.
    label_key = tuple((0, int(part)) if part.isdecimal() else (1, part)
                      for part in re.findall(r"\d+|\D+", label))
    return (prefix, base, 0 if label else 1, label_key)


def entries(releases):
    lines = []
    for release in releases:
        if release["draft"]:
            continue
        names = {asset["name"] for asset in release["assets"]}
        if "SHA256SUMS.sig" in names:
            continue
        for asset in release["assets"]:
            digest = asset.get("digest") or ""
            if not digest.startswith("sha256:"):
                raise SystemExit(f"{release['tag_name']}/{asset['name']}: GitHub gives no SHA-256")
            lines.append((tag_key(release["tag_name"]), release["tag_name"], asset["name"], digest[7:].lower()))
    return [f"{tag} {name} {hex}" for _, tag, name, hex in sorted(lines)]


def render(releases):
    return HEADER + "".join(line + "\n" for line in entries(releases))


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--check", action="store_true", help="exit 1 if the file is out of date, change nothing")
    args = parser.parse_args()
    out = subprocess.run(["gh", "api", f"repos/{REPO}/releases?per_page=100", "--paginate", "--slurp"],
                         capture_output=True, text=True, check=True).stdout
    text = render([release for page in json.loads(out) for release in page])
    if args.check:
        current = OUT.read_text() if OUT.exists() else ""
        if current != text:
            print(f"{OUT.relative_to(ROOT)} is out of date: run tools/ci/legacy_releases.py", file=sys.stderr)
            return 1
        print(f"{OUT.relative_to(ROOT)}: up to date ({text.count(chr(10)) - HEADER.count(chr(10))} files)")
        return 0
    OUT.write_text(text)
    print(f"wrote {OUT.relative_to(ROOT)}: {text.count(chr(10)) - HEADER.count(chr(10))} files")
    return 0


if __name__ == "__main__":
    sys.exit(main())
