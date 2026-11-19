"""Release selection must see every changed path Git records, including names Git quotes.

The regression exercises the real tools.release_train.changed_paths against synthetic Git
repositories owned by this test (TemporaryDirectory), never the working tree's repository
or its Git configuration.
"""

import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock

from tools import release_train as train


class ChangedPathsQuotedNameTests(unittest.TestCase):
    def git(self, repo, *args):
        result = subprocess.run(["git", "-c", "core.hooksPath=" + os.devnull, "-c", "commit.gpgSign=false",
                                 "-C", str(repo), *args], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, f"git {args}: {result.stderr}")
        return result.stdout

    def test_changed_paths_preserves_quoted_and_special_filenames(self):
        # Independent expected values: the exact names committed below. os.fsdecode is the
        # same filesystem-safe decoding Python uses for on-disk names, so a non-UTF-8 byte
        # round-trips as a surrogate-escaped str rather than being dropped or mangled.
        text_paths = [
            "launcher/src/main.rs",             # ordinary positive control
            "launcher/src/中文.rs",              # non-ASCII -> quoted with core.quotePath=true
            "launcher/src/my file.rs",          # space -> split into two tokens by the old code
        ]
        byte_paths = []
        if os.name != "nt":
            text_paths += ["launcher/src/tab\there.rs", "launcher/src/newline\nname.rs"]
            byte_paths = [b"launcher/src/bad_\xff.rs"]  # POSIX-only names; keep portable controls on Windows.
        expected = [os.fsdecode(p) for p in text_paths] + [os.fsdecode(p) for p in byte_paths]
        non_player = "tools/test_tool.py"

        for quote_path in ("true", "false"):
            with self.subTest(core_quotePath=quote_path), \
                    tempfile.TemporaryDirectory() as tmp:
                repo = Path(tmp)
                self.git(repo, "init", "-q")
                self.git(repo, "config", "user.email", "fixture@example.invalid")
                self.git(repo, "config", "user.name", "Fixture")
                self.git(repo, "config", "core.quotePath", quote_path)

                (repo / "README.md").write_text("base\n", encoding="utf-8")
                self.git(repo, "add", "-A")
                self.git(repo, "commit", "-qm", "base")

                for rel in text_paths:
                    full = repo / rel
                    full.parent.mkdir(parents=True, exist_ok=True)
                    full.write_text("// fixture\n", encoding="utf-8")
                for rel in byte_paths:
                    full = os.path.join(os.fsencode(str(repo)), rel)
                    os.makedirs(os.path.dirname(full), exist_ok=True)
                    with open(full, "wb") as stream:
                        stream.write(b"// fixture\n")
                (repo / "tools").mkdir(exist_ok=True)
                (repo / non_player).write_text("pass\n", encoding="utf-8")
                self.git(repo, "add", "-A")
                self.git(repo, "commit", "-qm", "change")
                head = self.git(repo, "rev-parse", "HEAD").strip()

                with mock.patch.object(train, "ROOT", repo):
                    changed = train.changed_paths(head)

                self.assertEqual(len(changed), len(expected) + 1, changed)
                self.assertEqual(set(changed), set(expected + [non_player]), changed)
                # Player-facing classification is unchanged and still sees these paths.
                for path in expected:
                    self.assertTrue(train.player_facing(path, "launcher"), path)
                self.assertFalse(train.player_facing(non_player, "launcher"))


if __name__ == "__main__":
    unittest.main()
