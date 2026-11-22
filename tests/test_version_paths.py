"""No tool writes the pinned Minecraft version by hand: every path comes from the lock file.

`tools/lab.py` says it: "The Minecraft version research-lock.json pins: every tool reads it from
there, never by hand." A path that spells the version out breaks on the next version bump, and
nothing catches it, because the tools that break are the ones that need the vanilla data to run.

The few places that name a version on purpose (a tool that compares two versions, tests with a lock file of their own) are
listed in ALLOWED, each with the reason.
"""
import json
from pathlib import Path
import re
import unittest

from tools import lab

ROOT = Path(__file__).resolve().parents[1]
VERSION = lab.VERSION

# Files that name a version on purpose, and why.
ALLOWED = {
    "tools/datapack_upgrade_fidelity.py": "compares 26.2 with 26.3 on purpose",
    "tests/test_lab.py": "builds a lock file with a version of its own",
    "tests/test_port_regen.py": "builds a lock file with a version of its own",
    "tests/test_compare_prepare_contract.py": "builds fake prepared trees with versioned directories on purpose",
    "tests/test_compare_prepare_work.py": "builds fake prepared trees with versioned directories on purpose",
    "tests/test_preparation_step.py": "builds fake source trees with versioned directories on purpose",
    "tools/ci/test_prepared_cache.py": "builds a fake prepared cache tree with versioned directories on purpose",
}

# A path that names the pinned version: a quoted literal that holds a separator, names the version
# as a segment of it, and has no spaces (a path has no spaces). Prose such as "vanilla 26.3" in a
# docstring is not a path.
PATH_LITERAL = re.compile(r"""['"][^'"]*/[^'"]*""" + re.escape(VERSION) + r"""[^'"]*['"]""")
# The version on its own, next to a path: `'horizon-client' / '26.3'`.
VERSION_ALONE = re.compile(r"""['"]""" + re.escape(VERSION) + r"""['"]""")
# The words that make a literal a path rather than prose.
PATH_WORDS = re.compile(r"local|minecraft|horizon-client|resources|\.jar|vanilla-")


def path_literal(line):
    """The path the line spells out, when it spells one."""
    for literal in PATH_LITERAL.finditer(line):
        text = literal.group(0)[1:-1]
        if " " in text:
            continue                      # prose inside a string
        if not PATH_WORDS.search(text):
            continue                      # a version in a word, not a path
        return literal.group(0)
    if VERSION_ALONE.search(line) and PATH_WORDS.search(line):
        return VERSION_ALONE.search(line).group(0)
    return None


def sources():
    for path in sorted(list((ROOT / "tools").rglob("*.py")) + list((ROOT / "tests").rglob("*.py"))):
        if path == Path(__file__).resolve():
            continue                   # this file names the version in its own comments
        yield path, path.read_text().splitlines()


class VersionPathsTests(unittest.TestCase):
    def test_the_lock_file_names_the_pinned_version(self):
        pinned = json.loads((ROOT / "research-lock.json").read_text())["minecraft"]["version"]
        self.assertEqual(lab.VERSION, pinned)

    def test_no_tool_writes_the_pinned_version_in_a_path(self):
        offenders = []
        for path, lines in sources():
            relative = path.relative_to(ROOT).as_posix()
            if relative in ALLOWED:
                continue
            for number, line in enumerate(lines, 1):
                literal = path_literal(line)
                if literal is None:
                    continue
                offenders.append(f"{relative}:{number}: {line.strip()[:160]}")
        self.assertEqual(offenders, [],
                         "these paths spell the pinned version out; use lab.RESOURCES, lab.GAME "
                         "or lab.VERSION instead:\n" + "\n".join(offenders))

    def test_the_allowed_files_still_exist(self):
        """The exceptions are a list of files, not a wildcard that can grow unnoticed."""
        for relative in ALLOWED:
            self.assertTrue((ROOT / relative).is_file(), relative)


if __name__ == "__main__":
    unittest.main()
