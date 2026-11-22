#!/usr/bin/env python3
"""Write tools/lang_unicode_tables.json, the character tables the translation text rules read
(docs/translations.md, "Rules"), from three files of the Unicode Character Database:

    python3 -I tools/gen_lang_unicode_tables.py \
        UnicodeData.txt Scripts.txt GraphemeBreakProperty.txt

The UCD files are not committed: download them from https://www.unicode.org/Public/<version>/ucd/
(GraphemeBreakProperty.txt is under auxiliary/) into an empty folder outside the repository. The
output records their version and SHA-256.
tools/lang_coverage.py and lang.rs's tests read only the output, so both apply the same tables.
"""
from __future__ import annotations

import hashlib
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUTPUT = ROOT / "tools/lang_unicode_tables.json"
# Scripts written without spaces between words. The game's wrapper breaks a line only at a space
# or a line break and cuts a longer one where it overflows, so their letters make no unbroken word.
SPACELESS_SCRIPTS = {"Han", "Hiragana", "Katakana", "Hangul", "Thai", "Lao", "Khmer", "Myanmar"}
LETTERS = {"Lo", "Lm"}
# Of Hangul, only the precomposed syllables: jamo and fillers are no syllable on their own.
HANGUL_SYLLABLES = {"LV", "LVT"}
MARKS = {"Mn", "Me"}
# Grapheme_Cluster_Break values that attach to the character before them like a mark does.
EXTENDING = {"Extend", "ZWJ"}
ABOUT = ("Inclusive, sorted, disjoint code point ranges, written by "
         "tools/gen_lang_unicode_tables.py from Unicode {version} ({sources}); "
         "tools/lang_coverage.py and lang.rs's tests read only this file. \"spaceless\": letters "
         "(Lo, Lm) of the scripts written without spaces (Han, Hiragana, Katakana, Thai, Lao, Khmer, Myanmar) and Hangul "
         "syllables (Grapheme_Cluster_Break LV, LVT); they don't count towards a token. "
         "\"combining\": general category Mn and Me; at most a few in a row. \"transparent\": "
         "format characters (Cf) and the other characters of Grapheme_Cluster_Break Extend or ZWJ "
         "(emoji modifiers, some spacing marks); they neither count as a combining mark nor end a "
         "row of them.")


def property_lines(path: Path):
    """(low, high, value) for each data line of a UCD property file such as Scripts.txt."""
    for line in path.read_text(encoding="utf-8").splitlines():
        line = line.split("#", 1)[0].strip()
        if not line:
            continue
        points, value = (part.strip() for part in line.split(";"))
        low, _, high = points.partition("..")
        yield int(low, 16), int(high or low, 16), value


def unicode_data(path: Path) -> dict[int, tuple[str, str]]:
    """Code point to (name, general category); a `<..., First>` and `<..., Last>` pair stands for
    every code point between them."""
    found, first = {}, None
    for line in path.read_text(encoding="utf-8").splitlines():
        fields = line.split(";")
        code, name, category = int(fields[0], 16), fields[1], fields[2]
        if name.endswith(", First>"):
            first = code
            continue
        if name.endswith(", Last>"):
            assert first is not None, line
            found.update({c: (name, category) for c in range(first, code + 1)})
            first = None
            continue
        found[code] = (name, category)
    return found


def ranges(points) -> list[list[int]]:
    out: list[list[int]] = []
    for c in sorted(points):
        if out and out[-1][1] == c - 1:
            out[-1][1] = c
        else:
            out.append([c, c])
    return out


def version(path: Path) -> str:
    first_line = path.read_text(encoding="utf-8").split("\n", 1)[0]
    match = re.match(r"# \S+-(\d+\.\d+\.\d+)\.txt", first_line)
    if not match:
        sys.exit(f"{path}: no version in its first line")
    return match.group(1)


def tables(data_path: Path, scripts_path: Path, breaks_path: Path) -> dict:
    data = unicode_data(data_path)
    scripts = {c: value for low, high, value in property_lines(scripts_path)
               for c in range(low, high + 1)}
    breaks = {c: value for low, high, value in property_lines(breaks_path)
              for c in range(low, high + 1)}
    spaceless = {c for c, (_, category) in data.items() if category in LETTERS
                 and scripts.get(c) in SPACELESS_SCRIPTS
                 and (scripts[c] != "Hangul" or breaks.get(c) in HANGUL_SYLLABLES)}
    combining = {c for c, (_, category) in data.items() if category in MARKS}
    transparent = {c for c, (_, category) in data.items() if category == "Cf"}
    transparent |= {c for c, value in breaks.items() if value in EXTENDING}
    transparent -= combining
    if spaceless & (combining | transparent):
        sys.exit("a spaceless letter is also a mark or transparent")
    versions = {version(scripts_path), version(breaks_path)}
    if len(versions) != 1:
        sys.exit(f"the files are of different Unicode versions: {sorted(versions)}")
    unicode = versions.pop()
    sources = {path.name: hashlib.sha256(path.read_bytes()).hexdigest()
               for path in (data_path, scripts_path, breaks_path)}
    return {
        "about": ABOUT.format(version=unicode, sources=", ".join(
            f"{name} SHA-256 {digest}" for name, digest in sources.items())),
        "unicode": unicode,
        "sources": sources,
        "spaceless": ranges(spaceless),
        "combining": ranges(combining),
        "transparent": ranges(transparent),
    }


def render(body: dict) -> str:
    """JSON with eight ranges to a line, so a regenerated table diffs by line."""
    lines = ["{"]
    for key in ("about", "unicode", "sources"):
        lines.append(f"  {json.dumps(key)}: {json.dumps(body[key], ensure_ascii=False)},")
    names = ("spaceless", "combining", "transparent")
    for key in names:
        rows = [", ".join(json.dumps(r) for r in body[key][i:i + 8])
                for i in range(0, len(body[key]), 8)]
        lines.append(f"  {json.dumps(key)}: [")
        lines.append(",\n".join(f"    {row}" for row in rows))
        lines.append("  ]" + ("," if key != names[-1] else ""))
    lines.append("}")
    return "\n".join(lines) + "\n"


def main(argv: list[str]) -> None:
    if len(argv) != 3:
        sys.exit(__doc__)
    OUTPUT.write_text(render(tables(*(Path(arg) for arg in argv))), encoding="utf-8", newline="\n")


if __name__ == "__main__":
    main(sys.argv[1:])
