#!/usr/bin/env python3
"""Extract an author's additive monolithic ingredient tests into a discovered family.

Run once per member, before hand-written rollup carries, for example:

    python3 tools/refactors/extract_builtin_ingredient_member.py \
        --base 42c5f703e3a0414297c1e1f00d62d4d4b4b2e580 \
        --head e235720b0791244d568251bb28c39963e17cef3e --member 4222

The base is the rollup's initial main, not a moving remote ref. Only functions
new since the graph merge base are copied, in author order and byte-for-byte.
Existing main test families are never rewritten. The existing build-time family
discovery picks up the output; no module list or selector registry is changed.
This uses the same top-level function/attribute segmentation convention as
split_builtin_ingredient_tests.py, which originally split main's monolith.
"""

import argparse
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parents[2]
SOURCE = 'crates/horizon-client/src/crafting/ingredient/builtin_kinds_tests.rs'
FAMILIES = ROOT / Path(SOURCE).with_suffix('') / 'families'


def git(*args):
    return subprocess.check_output(['git', *args], cwd=ROOT, text=True)


def functions(source):
    starts = list(re.finditer(r'(?m)^((?:#\[[^\n]*\]\n)*)fn (\w+)\(', source))
    result = {}
    for index, match in enumerate(starts):
        end = starts[index + 1].start() if index + 1 < len(starts) else len(source)
        name = match[2]
        if name in result:
            raise ValueError(f'duplicate top-level function: {name}')
        result[name] = source[match.start():end]
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--base', required=True, help='initial rollup main commit')
    parser.add_argument('--head', required=True, help='unchanged real author head')
    parser.add_argument('--member', required=True, type=int, help='pull request number')
    parser.add_argument('--output', type=Path, help='alternate output for reproduction checks')
    args = parser.parse_args()
    base = git('merge-base', args.base, args.head).strip()
    before = functions(git('show', f'{base}:{SOURCE}'))
    after = functions(git('show', f'{args.head}:{SOURCE}'))
    added = [body for name, body in after.items() if name not in before]
    if not added:
        parser.error('head adds no top-level ingredient test functions')
    output = args.output or FAMILIES / f'member_{args.member}.rs'
    content = 'use super::*;\n\n' + ''.join(added)
    if output.exists() and output.read_text() != content:
        parser.error(f'{output} differs; refusing to overwrite hand-written carries')
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(content)
    print(f'#{args.member}: extracted {len(added)} functions to {output}')


if __name__ == '__main__':
    main()
