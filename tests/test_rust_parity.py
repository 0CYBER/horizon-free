import json
from pathlib import Path
import tempfile
import unittest
from unittest import mock

from tools import test_rust_parity as parity


class ParityRunnerTests(unittest.TestCase):
    def test_report_rejects_sources_changed_during_a_run(self):
        with tempfile.TemporaryDirectory() as temp:
            # Resolved, as source_hashes resolves the sources: macOS's temp folder is a symlink
            # (/var -> /private/var).
            root = Path(temp).resolve()
            source = root / "source.rs"
            source.write_text("before")
            with mock.patch.object(parity.lab, "ROOT", root):
                before = parity.source_hashes([source])
                parity.ensure_sources_unchanged([source], before)
                source.write_text("after")
                with self.assertRaisesRegex(RuntimeError, "source.rs"):
                    parity.ensure_sources_unchanged([source], before)
    def test_sources_under_a_linked_local_folder_are_named_within_the_repository(self):
        # A worktree whose `.local` links to another checkout's: the harness's inputs live under
        # that link, and resolving them would leave the repository.
        with tempfile.TemporaryDirectory() as temp:
            root, elsewhere = Path(temp).resolve() / "worktree", Path(temp).resolve() / "shared"
            root.mkdir()
            (elsewhere / "rust-x-parity").mkdir(parents=True)
            try:
                (root / ".local").symlink_to(elsewhere, target_is_directory=True)
            except OSError as e:
                # Windows lets anyone make a junction, which is what a worktree there uses.
                try:
                    import _winapi
                    _winapi.CreateJunction(str(elsewhere), str(root / ".local"))
                except (ImportError, OSError):
                    self.skipTest(f"no directory links here: {e}")
            inputs = root / ".local" / "rust-x-parity" / "inputs.txt"
            inputs.write_text("1 2 3\n")
            with mock.patch.object(parity.lab, "ROOT", root):
                hashes = parity.source_hashes([inputs])
            self.assertEqual(list(hashes), [str(Path(".local", "rust-x-parity", "inputs.txt"))])
            with mock.patch.object(parity.lab, "ROOT", root), self.assertRaisesRegex(ValueError, "outside"):
                parity.source_hashes([Path(temp) / "stray.txt"])

    def test_comparison_rejects_changed_missing_extra_and_empty_results(self):
        for reference, candidate in [("rng 0 1\n", "rng 0 2\n"),
                                     ("rng 0 1\n", ""),
                                     ("rng 0 1\n", "rng 0 1\nrng 1 2\n"), ("", "")]:
            with self.subTest(reference=reference, candidate=candidate), tempfile.TemporaryDirectory() as temp:
                left, right = Path(temp) / "reference", Path(temp) / "candidate"
                left.write_text(reference)
                right.write_text(candidate)
                with self.assertRaises(AssertionError):
                    parity.compare(left, right)

    def test_comparison_counts_matching_lines(self):
        with tempfile.TemporaryDirectory() as temp:
            left, right = Path(temp) / "reference", Path(temp) / "candidate"
            left.write_text("rng 0 1\nrng 1 -1\n")
            right.write_text(left.read_text())
            self.assertEqual(parity.compare(left, right), 2)

    def test_chunk_comparison_preserves_newlines_unicode_and_unterminated_lines(self):
        cases = [
            ("one", "one", 1),
            ("\n\n", "\n\n", 2),
            ("one\r\ntwo\r", "one\ntwo\n", 2),
            ("α" * (1024 * 1024 + 1) + "\r\nlast", "α" * (1024 * 1024 + 1) + "\nlast", 2),
            ("same\n" * 220000 + "tail", "same\n" * 220000 + "tail", 220001),
        ]
        with tempfile.TemporaryDirectory() as temp:
            left, right = Path(temp) / "reference", Path(temp) / "candidate"
            for reference, candidate, count in cases:
                left.write_bytes(reference.encode("utf-8"))
                right.write_bytes(candidate.encode("utf-8"))
                self.assertEqual(parity.compare(left, right), count)

    def test_chunk_mismatch_retains_exact_first_line_diagnostic(self):
        prefix = "same\n" * 220000
        cases = [("expected\n", "actual\n", "'expected\\n'", "'actual\\n'"),
                 ("tail", "tail\n", "'tail'", "'tail\\n'"),
                 ("extra\n", "", "'extra\\n'", "None")]
        with tempfile.TemporaryDirectory() as temp:
            left, right = Path(temp) / "reference", Path(temp) / "candidate"
            for reference, candidate, expected, actual in cases:
                left.write_text(prefix + reference)
                right.write_text(prefix + candidate)
                with self.assertRaises(AssertionError) as caught:
                    parity.compare(left, right)
                self.assertEqual(str(caught.exception),
                                 f"Parity mismatch at output line 220001: vanilla={expected}, Rust={actual}")

    def mismatch(self, reference, candidate, context=5):
        with tempfile.TemporaryDirectory() as temp:
            left, right = Path(temp) / "reference", Path(temp) / "candidate"
            left.write_bytes(reference if isinstance(reference, bytes) else reference.encode("utf-8"))
            right.write_bytes(candidate if isinstance(candidate, bytes) else candidate.encode("utf-8"))
            with self.assertRaises(AssertionError) as caught:
                parity.compare(left, right, context=context)
        return str(caught.exception)

    def test_context_shows_lines_around_a_mismatch_in_the_middle(self):
        same = "".join(f"read {i} 0 0\n" for i in range(1, 11))
        message = self.mismatch(same + "survive false\n" + "counts 1 2 3 4\n" * 8,
                                same + "read 0 -1 0\n" + "survive true\n" + "counts 1 2 3 4\n" * 2)
        lines = message.split("\n")
        self.assertEqual(lines[0], "Parity mismatch at output line 11: vanilla='survive false\\n', Rust='read 0 -1 0\\n'")
        self.assertEqual(lines[1:], [
            "Matching lines before it (5, the same on both sides):",
            "             6  'read 6 0 0\\n'",
            "             7  'read 7 0 0\\n'",
            "             8  'read 8 0 0\\n'",
            "             9  'read 9 0 0\\n'",
            "            10  'read 10 0 0\\n'",
            "vanilla from line 11:",
            "  >         11  'survive false\\n'",
            *[f"    {n:>10}  'counts 1 2 3 4\\n'" for n in range(12, 17)],
            "Rust from line 11:",
            "  >         11  'read 0 -1 0\\n'",
            "            12  'survive true\\n'",
            "            13  'counts 1 2 3 4\\n'",
            "            14  'counts 1 2 3 4\\n'",
            "            15  (end of output)",
            "  (two '#' before '[' and two ':' in a row are shown with a space between them)",
        ])

    def test_context_at_the_first_and_last_line_and_with_a_shorter_side(self):
        message = self.mismatch("a\nb\n", "x\nb\n")
        self.assertIn("No matching lines before it.", message)
        self.assertIn("  >          1  'a\\n'\n             2  'b\\n'\n             3  (end of output)", message)
        message = self.mismatch("a\nb\nc\n", "a\nb\nd\n")
        self.assertIn("  >          3  'c\\n'\n             4  (end of output)\nRust", message)
        message = self.mismatch("a\nb\nc\nd\n", "a\nb\n")
        self.assertTrue(message.startswith("Parity mismatch at output line 3: vanilla='c\\n', Rust=None\n"))
        self.assertIn("Rust from line 3:\n  >          3  (end of output)", message)
        self.assertIn("  >          3  'c\\n'\n             4  'd\\n'\n             5  (end of output)", message)

    def test_context_cuts_long_lines_and_the_whole_block(self):
        long = "z" * 1_000_000
        message = self.mismatch(f"a\n{long}\nb\n{long}\n", f"a\n{long}\nc\n{long}\n")
        self.assertIn("             2  '" + "z" * 200 + "' (999,801 more characters cut)", message)
        self.assertIn("             4  '" + "z" * 200 + "' (999,801 more characters cut)", message)
        self.assertNotIn("z" * 201, message)
        lines = "".join(f"{'y' * 300} {i}\n" for i in range(20))
        message = self.mismatch(lines + "a\n" + lines, lines + "b\n" + lines, context=50)
        block = message.split("\n", 1)[1]
        self.assertLessEqual(len(block.encode("utf-8")), parity.CONTEXT_BLOCK_LIMIT)
        self.assertIn("  (context cut to 4,096 bytes)", block)
        self.assertIn("  >         21  'a\\n'", block)
        self.assertIn("  >         21  'b\\n'", block)

    def test_context_cut_keeps_both_mismatching_rows(self):
        heavy = ["\x01" * 150 + "€" * 50 + f" {i}\n" for i in range(12)]
        before, after = "".join(heavy[:5]), "".join(heavy[5:])
        message = self.mismatch(before + "\x02" * 197 + "a\n" + after, before + "€" * 197 + "b\n" + after)
        block = message.split("\n", 1)[1]
        self.assertLessEqual(len(("\n" + block).encode("utf-8")), parity.CONTEXT_BLOCK_LIMIT)
        self.assertIn("  >          6  '" + "\\x02" * 197 + "a\\n'", block)
        self.assertIn("  >          6  '" + "€" * 197 + "b\\n'", block)
        self.assertIn("  (context cut to 4,096 bytes)", block)
        # The lines before go first: all five, before any line after.
        self.assertIn("Matching lines before it (0 of 5 shown, the same on both sides):", block)
        self.assertIn("\n             7  ", block)

    def test_context_spaces_workflow_command_sequences(self):
        same = "##[warning]x\n::error::x\n"
        tail = "::error::x\n##[error]x\n:::x\n"
        message = self.mismatch(same + "##[error]x\n" + tail, same + "::error::x\n" + tail)
        first, block = message.split("\n", 1)
        self.assertEqual(first, "Parity mismatch at output line 3: vanilla='##[error]x\\n', Rust='::error::x\\n'")
        self.assertNotIn("##[", block)
        self.assertNotIn("::", block)
        for shown in ["'# #[warning]x\\n'", "': :error: :x\\n'", "'# #[error]x\\n'", "': : :x\\n'"]:
            self.assertIn(shown, block)
        self.assertIn("  >          3  '# #[error]x\\n'", block)
        self.assertIn("  >          3  ': :error: :x\\n'", block)
        self.assertTrue(block.endswith("are shown with a space between them)"))
        self.assertEqual(self.mismatch(same + "##[error]x\n", same + "::error::x\n", context=0), first)

    def test_bad_bytes_after_the_mismatch_do_not_hide_it(self):
        padding = b"z" * 1_500_000
        first = "Parity mismatch at output line 2: vanilla='X\\n', Rust='Y\\n'"
        reference, candidate = b"a\nX\n" + padding + b"\xff\nq\n", b"a\nY\n" + padding + b"\xff\nq\n"
        self.assertEqual(self.mismatch(reference, candidate, context=0), first)
        message = self.mismatch(reference, candidate)
        self.assertTrue(message.startswith(first + "\n"))
        self.assertIn("             3  '" + "z" * 200 + "' (1,499,805 more characters cut)\n             4  'q\\n'", message)
        # A bad byte inside a shown line is shown escaped.
        reference, candidate = b"a\nX\n" + padding + b"\nbad \xff x\n", b"a\nY\n" + padding + b"\nbad \xff x\n"
        self.assertEqual(self.mismatch(reference, candidate, context=0), first)
        message = self.mismatch(reference, candidate)
        self.assertTrue(message.startswith(first + "\n"))
        self.assertIn("             4  'bad \\\\xff x\\n'", message)

    def test_context_cannot_start_workflow_commands_or_terminal_escapes(self):
        hostile = "::error::owned\n\x1b[2J\x1b]8;;x\x07\n"
        message = self.mismatch(hostile + "a\n" + hostile, hostile + "b\n" + hostile)
        for line in message.split("\n")[1:]:
            self.assertFalse(line.startswith("::"), line)
            self.assertTrue(line == "" or line.startswith(" ") or line.endswith(":"), line)
        self.assertNotIn("\x1b", message)
        self.assertNotIn("\x07", message)
        self.assertIn("'\\x1b[2J\\x1b]8;;x\\x07\\n'", message)

    def test_context_streams_large_traces(self):
        import tracemalloc
        line = "x" * 99 + "\n"
        with tempfile.TemporaryDirectory() as temp:
            left, right = Path(temp) / "reference", Path(temp) / "candidate"
            for path, last in [(left, "a\n"), (right, "b\n")]:
                with path.open("w", encoding="utf-8", newline="") as out:
                    for _ in range(40):
                        out.write(line * 5000)
                    out.write(last + line * 10)
            size = left.stat().st_size
            tracemalloc.start()
            try:
                with self.assertRaises(AssertionError) as caught:
                    parity.compare(left, right, context=5)
                peak = tracemalloc.get_traced_memory()[1]
            finally:
                tracemalloc.stop()
        self.assertIn("at output line 200001:", str(caught.exception))
        self.assertGreater(size, 20_000_000)
        self.assertLess(peak, size // 2)

    def test_failed_artifact_verification_removes_previous_success(self):
        with tempfile.TemporaryDirectory() as temp:
            local = Path(temp)
            report = local / "rust-parity/report.json"
            report.parent.mkdir()
            report.write_text(json.dumps({"previous_success": True}))
            with mock.patch.object(parity.lab, "LOCAL", local), mock.patch("sys.argv", ["parity"]), \
                 mock.patch.object(parity, "verify_artifacts", side_effect=ValueError("corrupt artifact")):
                with self.assertRaisesRegex(ValueError, "corrupt artifact"):
                    parity.main()
            self.assertFalse(report.exists())

    def test_corpus_is_repeatable_and_contains_rejection_and_word_boundaries(self):
        commands = list(parity.corpus())
        self.assertEqual(commands, list(parity.corpus()))
        self.assertIn("xoroshiro_state 0 0 2200", commands)
        self.assertIn("legacy -9223372036854775808 2200", commands)
        factories = [line.split() for line in commands if line.startswith("factory ")]
        self.assertEqual(len(factories), 4400)
        self.assertTrue({"-", "d800", "dc00", "d83dde00", "d8000061dc00"} <= {line[9] for line in factories})
        helpers = [line.split() for line in commands if line.startswith("helpers ")]
        self.assertEqual(len(helpers), 5000)
        for stream in ["legacy", "xoroshiro"]:
            bounds = {(int(line[3]), int(line[4])) for line in helpers if line[1] == stream}
            self.assertTrue({(-2147483648, 2147483647), (2147483647, -2147483648), (0, 0), (0, 1073741825)} <= bounds)
        for bits in range(1, 33):
            lengths = {int(line.split()[2]) for line in commands if line.startswith(f"storage {bits} ")}
            per_word = 64 // bits
            self.assertTrue({0, 1, per_word - 1, per_word, per_word + 1, 4096} <= lengths)


if __name__ == "__main__":
    unittest.main()
