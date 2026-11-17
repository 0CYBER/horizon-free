"""Parity harnesses name repository files and the game binary's package; both broke silently when the mod API merge (#17145) moved files and the binary."""
import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
# A quoted repository path, not part of an f-string placeholder or a longer name.
REPO_PATH = re.compile(r"""(?<![{\w])['"]((?:crates|mods|tools)/[\w./-]+\.(?:rs|java|py|toml))['"]""")
CLIENT_BUILD = re.compile(r"""\[[^\[\]]*?['"]--bin['"]\s*,\s*['"]horizon-client['"][^\[\]]*?\]""")
PACKAGE = re.compile(r"""['"]-p['"]\s*,\s*['"]([\w-]+)['"]""")


class ParityHarnessSources(unittest.TestCase):
    def test_named_source_files_exist(self):
        # The harnesses hash these files into their cache keys, so a moved file stops the run.
        missing = [f"{script.name}: {path}"
                   for script in sorted((ROOT / "tools").glob("test_*_parity.py"))
                   for path in REPO_PATH.findall(script.read_text(encoding="utf-8"))
                   if not (ROOT / path).exists()]
        self.assertEqual(missing, [])

    def test_client_binary_is_built_from_its_package(self):
        builds = [(script.name, command)
                  for script in sorted((ROOT / "tools").glob("**/*.py"))
                  for command in CLIENT_BUILD.findall(script.read_text(encoding="utf-8"))]
        self.assertTrue(builds)
        wrong = [name for name, command in builds
                 if "horizon-game" not in PACKAGE.findall(command)]
        self.assertEqual(wrong, [])


if __name__ == "__main__":
    unittest.main()
