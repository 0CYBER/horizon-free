import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from tools.ci import format_scan  # noqa: E402


class FormatScanTest(unittest.TestCase):
    def test_cargo_fmt_then_every_file_no_mod_line_reaches_by_path(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for path in ("crates/horizon-client/src/game/world/levels.rs",
                         "crates/horizon-client/src/game/world/levels/tests.rs",
                         "crates/horizon-mod-api/src/lib.rs",
                         "crates/horizon-mod-vocab/src/hooks/player.rs",
                         "crates/horizon-mod-api-gen/dispatch-tests/src/fixtures/a.rs",
                         "crates/horizon-mod-api-gen/dispatch-tests/src/fixtures/b/c.rs",
                         "crates/horizon-mod-wasm/wire-loopback/tests/wire_loopback/d.rs",
                         # Reached by cargo fmt through its `mod` lines, so not listed by path.
                         "crates/horizon-client/src/lib.rs",
                         "crates/horizon-client/src/game/world/notes.md"):
                (root / path).parent.mkdir(parents=True, exist_ok=True)
                (root / path).write_text("")
            self.assertEqual(format_scan.commands(root), [
                ["cargo", "fmt", "--all", "--", "--check"],
                ["rustfmt", "--edition", "2024", "--check", "--config", "skip_children=true",
                 "crates/horizon-client/src/game/world/levels.rs",
                 "crates/horizon-client/src/game/world/levels/tests.rs",
                 "crates/horizon-mod-api/src/lib.rs",
                 "crates/horizon-mod-vocab/src/hooks/player.rs",
                 "crates/horizon-mod-api-gen/dispatch-tests/src/fixtures/a.rs",
                 "crates/horizon-mod-api-gen/dispatch-tests/src/fixtures/b/c.rs",
                 "crates/horizon-mod-wasm/wire-loopback/tests/wire_loopback/d.rs"],
            ])

    def test_long_file_lists_are_split_and_every_check_runs(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            game = root / "crates/horizon-client/src/game"
            game.mkdir(parents=True)
            for index in range(format_scan.CHUNK + 1):
                (game / f"m{index:03}.rs").write_text("")
            commands = format_scan.commands(root)
            self.assertEqual([len(command) - len(format_scan.RUSTFMT) for command in commands[1:]],
                             [format_scan.CHUNK, 1])
            ran = []
            # The first rustfmt chunk fails; the second still runs, and the scan fails.
            with mock.patch.object(format_scan, "ROOT", root), \
                    mock.patch.object(format_scan.subprocess, "run",
                                      side_effect=lambda command, cwd: ran.append(command) or mock.Mock(
                                          returncode=1 if command == commands[1] else 0)):
                self.assertEqual(format_scan.main(), 1)
            self.assertEqual(ran, commands)


if __name__ == "__main__":
    unittest.main()
