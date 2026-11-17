#!/usr/bin/env python3
"""Reject self-mappings in changed rename prose or explicitly supplied notes."""
import re
import subprocess
import sys
from pathlib import Path

# Capture the bare identifier: any backticks live outside the group so the
# numeric backreference compares names, not their markdown decoration. The two
# sides of a mapping need not agree on backticks ("`Foo` becomes Foo" is still
# a self-mapping).
NAME = r"`?\b([A-Za-z_]\w*(?:::\w+)*)\b`?"
AGAIN = r"`?\b\1\b`?"  # the same name, independently backticked
SELF = re.compile(NAME + r"\s*(?:→|->|\b(?i:becomes)\b)\s*" + AGAIN + r"(?![\w:])")
RENAMED = re.compile(r"\b(?i:renamed)\s+" + NAME + r"\s+(?i:to)\s+" + AGAIN + r"(?![\w:])")
PAIR = re.compile(r"`?\b([A-Za-z_]\w*)\b`?\s*(?:\||(?i:and))\s*`?\b\1\b`?")


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
