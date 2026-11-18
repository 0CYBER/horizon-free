"""Escaped/plain/raw equivalence for the built-in address gate (#17305).

The Rust snippets here are parser inputs, not shipped source; every expected value below is an
independent literal. The out-of-home host is neither contacted nor added to any allowlist.
"""

import tempfile
import unittest
from pathlib import Path
from unittest import mock

from tools import check_addresses


REL = "crates/synthetic/src/lib.rs"
HOST = "play.playhorizon.dev"
DECODED_URL = "https://play.playhorizon.dev/ping"
ESCAPED_URL = r"https://play\x2eplayhorizon\x2edev/ping"
ERROR = [
    "crates/synthetic/src/lib.rs: play.playhorizon.dev is a built-in address outside the address list. "
    "A vanilla endpoint goes in 'vanilla', anything else in the Horizon layer's address home "
    "(docs/layers.md#built-in-addresses)"
]
CONFIG = {"homes": [], "vanilla": {}, "not_addresses": {}, "allowed": []}


def analyse(source):
    """(kept literals, hosts, policy problems, notes) for one synthetic Rust source."""
    literals, mask = check_addresses.tokenize(source)
    ranges, _ = check_addresses.test_items(source, mask)
    kept = [s for offset, s in literals if not any(a <= offset < b for a, b in ranges)]
    found = [h for s in kept for h in check_addresses.hosts(s)]
    problems, notes = check_addresses.check(CONFIG, {(REL, h) for h in found})
    return kept, found, problems, notes


class RustStringEscapeTests(unittest.TestCase):
    def test_escaped_plain_and_raw_spellings_give_the_same_host_and_policy(self):
        # (case, Rust source as parser input, decoded literals kept, hosts, policy problems)
        matrix = [
            ("plain control: the real out-of-home host error",
             f'const U: &str = "{DECODED_URL}";\n',
             [DECODED_URL], [HOST], ERROR),
            ("ASCII \\xNN escapes",
             'const U: &str = "https://play\\x2eplayhorizon\\x2edev/ping";\n',
             [DECODED_URL], [HOST], ERROR),
            ("\\u{...} escapes",
             'const U: &str = "https://play\\u{2e}playhorizon\\u{2e}dev/ping";\n',
             [DECODED_URL], [HOST], ERROR),
            ("\\u{...} with underscores between hex digits",
             'const U: &str = "https://play\\u{00_2e}playhorizon\\u{00_2e}dev/ping";\n',
             [DECODED_URL], [HOST], ERROR),
            ("\\u{...} with six hex digits and several underscores",
             'const U: &str = "https://play\\u{0_0_0_0_2_e}playhorizon.dev/ping";\n',
             [DECODED_URL], [HOST], ERROR),
            ("\\u{...} with multiple internal underscores (rustc-valid)",
             'const U: &str = "https://play\\u{00__2e}playhorizon.dev/ping";\n',
             [DECODED_URL], [HOST], ERROR),
            ("\\u{...} with a trailing underscore (rustc-valid)",
             'const U: &str = "https://play\\u{2e_}playhorizon.dev/ping";\n',
             [DECODED_URL], [HOST], ERROR),
            ("mixed \\xNN and \\u{...}",
             'const U: &str = "https://play\\x2eplayhorizon\\u{2e}dev/ping";\n',
             [DECODED_URL], [HOST], ERROR),
            ("backslash-newline ASCII whitespace continuation",
             'const U: &str = "https://play\\\n'
             '    .playhorizon.dev/ping";\n',
             [DECODED_URL], [HOST], ERROR),
            ("raw string keeps escape spellings literal",
             f'const U: &str = r"{ESCAPED_URL}";\n',
             [ESCAPED_URL], [], []),
            ("raw string holding a plain host is still a host",
             f'const U: &str = r"{DECODED_URL}";\n',
             [DECODED_URL], [HOST], ERROR),
            ("line and block comments stay excluded",
             f'// "{DECODED_URL}"\n/* "{DECODED_URL}" */\n',
             [], [], []),
            ("#[cfg(test)] module stays excluded",
             f'#[cfg(test)]\nmod tests {{\n    const U: &str = "{DECODED_URL}";\n}}\n',
             [], [], []),
            ("unrecognized escape is kept visible, not decoded away",
             'const U: &str = "https://play\\qplayhorizon.dev";\n',
             ["https://play\\qplayhorizon.dev"], [], []),
        ]
        for label, source, kept, found, problems in matrix:
            with self.subTest(label):
                actual_kept, actual_found, actual_problems, notes = analyse(source)
                self.assertEqual(actual_kept, kept)
                self.assertEqual(actual_found, found)
                self.assertEqual(actual_problems, problems)
                self.assertEqual(notes, [])

    def test_scan_of_files_with_only_escaped_spellings_reports_the_same_host_and_policy(self):
        """Whole-file gate: scan()'s prefilter must not skip a file whose only host spelling is escaped."""
        escaped_url = r"https\x3a\x2f\x2fplay\x2eplayhorizon\x2edev\x2fping"
        escaped_host = r"\x70lay.playhorizon.dev"
        cases = [
            ("plain control", f'const U: &str = "{DECODED_URL}";', {(REL, HOST)}, ERROR),
            ("fully escaped URL, no literal :// and no plain host",
             f'const U: &str = "{escaped_url}";', {(REL, HOST)}, ERROR),
            ("no-scheme escaped whole-string host",
             f'const U: &str = "{escaped_host}";', {(REL, HOST)}, ERROR),
            ("raw string keeps the escaped spelling literal",
             f'const U: &str = r"{escaped_url}";', set(), []),
        ]
        with tempfile.TemporaryDirectory(prefix="check_addresses_w11_") as tmp:
            root = Path(tmp)
            path = root / REL
            path.parent.mkdir(parents=True)
            with mock.patch.object(check_addresses, "ROOT", root):
                for label, source, expected_found, expected_problems in cases:
                    with self.subTest(label):
                        path.write_text(source + "\n", encoding="utf-8")
                        found = check_addresses.scan(files=[path])
                        self.assertEqual(found, expected_found)
                        problems, notes = check_addresses.check(CONFIG, found)
                        self.assertEqual(problems, expected_problems)
                        self.assertEqual(notes, [])


if __name__ == "__main__":
    unittest.main()
