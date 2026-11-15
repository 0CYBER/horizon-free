#!/usr/bin/env python3
"""One-shot, lossless extraction of Part fixture items into discovered fragments.

Run on the pre-split tree. Includes remain in the original lexical scope, so
fixture visibility, test paths and macro expansion scopes do not change.
"""
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]


def split(area):
    path = ROOT / f'crates/horizon-mod-api/src/parts/{area}.rs'
    source = path.read_text()
    start = source.index('pub mod tests {') + len('pub mod tests {\n')
    end = source.index('\n}\n', start)
    lines = source[start:end].splitlines(keepends=True)
    boundaries = []
    pattern = r'    (?:pub(?:\([^)]*\))? )?(?:async )?(fn|struct|enum|impl|type|static|const|thread_local!)(?: |\s*\{)(.*)'
    for i, line in enumerate(lines):
        match = re.match(pattern, line)
        if not match:
            continue
        j = i
        while j > 0 and lines[j - 1].strip() and (lines[j - 1].startswith('    #[') or lines[j - 1].startswith('    //')):
            j -= 1
        kind, rest = match.groups()
        name = re.search(r'\w+', rest)
        name = name.group() if name else 'storage'
        if kind == 'impl':
            name = re.search(r'for (\w+)', rest).group(1) if ' for ' in rest else name
        boundaries.append((j, name.lower()))
    directory = path.parent / f'{area}_fixtures'
    directory.mkdir()
    fragments = {}
    for n, (a, name) in enumerate(boundaries):
        b = boundaries[n + 1][0] if n + 1 < len(boundaries) else len(lines)
        fragments.setdefault(name, []).append(''.join(lines[a:b]))
    # Verify lossless extraction in original order before any writes.
    assert ''.join(lines[:boundaries[0][0]]) + ''.join(
        ''.join(lines[a:boundaries[n + 1][0] if n + 1 < len(boundaries) else len(lines)])
        for n, (a, _) in enumerate(boundaries)) == source[start:end]
    for name, pieces in fragments.items():
        (directory / f'{name}.rs').write_text(''.join(pieces))
    header = ''.join(lines[:boundaries[0][0]])
    path.write_text(source[:start] + header + f'    include!(concat!(env!("OUT_DIR"), "/{area}_fixtures.rs"));\n' + source[end:])


if __name__ == '__main__':
    for area in ('blocks', 'shared'):
        split(area)
