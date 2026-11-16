#!/usr/bin/env python3
"""Reject self-mappings in changed rename prose or explicitly supplied notes."""
import re
import subprocess
import sys
from pathlib import Path

NAME = r"(`?\b[A-Za-z_]\w*(?:::\w+)*`?)"
SELF = re.compile(NAME + r"\s*(?:→|->|\b(?i:becomes)\b)\s*\1(?![\w:])")
RENAMED = re.compile(r"\b(?i:renamed)\s+" + NAME + r"\s+(?i:to)\s+\1(?![\w:])")
PAIR = re.compile(r"`([A-Za-z_]\w*)`\s*(?:\||(?i:and))\s*`\1`")


def check(text):
    return [line for line in text.splitlines()
            if SELF.search(line) or RENAMED.search(line)
            or (PAIR.search(line) and ('|' in line or 'rename' in line.lower()))]


if __name__ == '__main__':
    changed = subprocess.check_output(['git', 'diff', '--unified=0', 'origin/main'], text=True)
    sources = [('diff', '\n'.join(line[1:] for line in changed.splitlines()
                                  if line.startswith('+') and not line.startswith('+++')))]
    sources += [(name, Path(name).read_text()) for name in sys.argv[1:]]
    errors = [(name, line) for name, text in sources for line in check(text)]
    for name, line in errors:
        print(f'{name}: self-mapping in rename prose: {line}', file=sys.stderr)
    sys.exit(bool(errors))
