#!/usr/bin/env python3
"""Bakes the static Bricolage Grotesque files in crates/horizon-theme/fonts from upstream's variable font.

GPUI picks a font file by family and weight, so the launcher can't use the variable font itself:
this cuts the instances it uses (docs/design-system.md, "Night"): the UI family at optical size 14
in Regular, Medium, SemiBold and Bold, and the display family (Bold at optical size 72) under its
own family name. Upstream: github.com/ateliertriay/bricolage, via google/fonts (SIL OFL 1.1, no
reserved font name).

    python3 -m venv /tmp/ft && /tmp/ft/bin/pip install fonttools
    /tmp/ft/bin/python tools/bake_bricolage.py 'BricolageGrotesque[opsz,wdth,wght].ttf'
"""

import sys
from pathlib import Path

from fontTools.ttLib import TTFont
from fontTools.varLib.instancer import instantiateVariableFont

OUT = Path(__file__).resolve().parent.parent / "crates/horizon-theme/fonts"
UI = "Bricolage Grotesque"
DISPLAY = "Bricolage Grotesque Display"
CUTS = [
    (UI, "Regular", 14, 400),
    (UI, "Medium", 14, 500),
    (UI, "SemiBold", 14, 600),
    (UI, "Bold", 14, 700),
    (DISPLAY, "Bold", 72, 700),
]


def rename(font: TTFont, family: str, style: str) -> None:
    """Names the instance as a plain static family: GPUI matches the family (1, 16) and weight."""
    full = f"{family} {style}"
    ps = f"{family.replace(' ', '')}-{style}"
    names = font["name"]
    names.names = [n for n in names.names if n.nameID not in (16, 17, 21, 22, 25)]
    for nid, value in [(1, family), (2, style), (3, ps), (4, full), (6, ps), (16, family), (17, style)]:
        names.setName(value, nid, 3, 1, 0x409)
        names.removeNames(nameID=nid, platformID=1)
    for table in ("STAT", "fvar", "avar"):
        if table in font:
            del font[table]


def main() -> None:
    source = Path(sys.argv[1])
    for family, style, opsz, wght in CUTS:
        font = TTFont(source)
        font = instantiateVariableFont(font, {"opsz": opsz, "wdth": 100, "wght": wght})
        rename(font, family, style)
        os2 = font["OS/2"]
        os2.usWeightClass = wght
        # fsSelection: bit 5 bold, bit 6 regular; macStyle bit 0 bold.
        os2.fsSelection = os2.fsSelection & ~0x61 | (0x20 if wght == 700 else 0x40 if wght == 400 else 0)
        font["head"].macStyle = 1 if wght == 700 else 0
        out = OUT / f"{family.replace(' ', '')}-{style}.ttf"
        font.save(out)
        print(f"{out.relative_to(OUT.parent.parent.parent)}: {out.stat().st_size // 1024} KiB")


if __name__ == "__main__":
    main()
