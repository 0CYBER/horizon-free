"""Parity harnesses name repository files and the game binary's package; both broke silently when the mod API merge (#17145) moved files and the binary."""
import ast
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

    def test_fast_programs_are_built_with_the_shared_selection(self):
        # tools/fast_bins.py: a fast program built with its own -p list compiles the game's chain once more in the
        # parity prebuild. Not parity harnesses, so the prebuild never builds their selection:
        allowed = {
            "mod_dimensions_e2e.py": "adds horizon-mod-dimension-game, a game with a test mod built in",
            "profile_client_e2e.py": "adds horizon-perf and builds every program of its packages",
        }
        # old_world_e2e.py builds an example in the same run, which the check leaves out: examples turn on
        # horizon-client-lib's dev-dependencies and must stay apart from the shared selection anyway.
        found = {script.name for script in sorted((ROOT / "tools").glob("**/*.py"))
                 for node in ast.walk(ast.parse(script.read_text(encoding="utf-8")))
                 if isinstance(node, ast.List) and hand_built_fast_program(node)}
        self.assertEqual(found - set(allowed), set())
        self.assertEqual(set(allowed) - found, set(), "no longer builds by hand: leave it out of the allowed list")


def hand_built_fast_program(node):
    """A literal Cargo command building programs in the fast profile with -p flags of its own, no example."""
    words = [element.value for element in node.elts if isinstance(element, ast.Constant) and isinstance(element.value, str)]
    fast = "--profile=fast" in words or any(a == "--profile" and b == "fast" for a, b in zip(words, words[1:]))
    return words[:1] == ["cargo"] and fast and bool({"-p", "--package"} & set(words)) \
        and bool({"--bin", "--bins"} & set(words)) and not {"--example", "--examples"} & set(words)


if __name__ == "__main__":
    unittest.main()
