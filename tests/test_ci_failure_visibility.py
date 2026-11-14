"""Synthetic diagnostics only: never use prepared resource output as a fixture."""
import contextlib
import io
import subprocess
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from tests.test_ci_concurrency import yaml, ROOT
from tests.test_ignored_test_completion import IgnoredTests, target, summary
from tools.ci import ignored_tests


class Visibility(unittest.TestCase):
    def test_panic_category_and_identity_without_operands(self):
        output = target(1, "test suite::broken ... FAILED\n"
                        "thread 'suite::broken' panicked at src/lib.rs:12:3:\n"
                        "assertion `left == right` failed: PRIVATE_RESOURCE\n"
                        " left: PRIVATE_RESOURCE\n", summary(failed=1))
        result, log = IgnoredTests().run_output(output, 101)
        self.assertEqual(result, {"suite::broken": False})
        self.assertIn("failed test: suite::broken", log)
        self.assertIn("assertion equality failed", log)
        self.assertNotIn("PRIVATE_RESOURCE", log)

    def test_unquoted_compiler_payloads_are_never_published(self):
        with contextlib.redirect_stdout(io.StringIO()) as log:
            ignored_tests.compile_diagnostics("error: PRIVATE_RESOURCE_CONTENT\n"
                                              "error[E0308]: PRIVATE_RESOURCE_CONTENT\n"
                                              "error: mismatched types PRIVATE_RESOURCE_CONTENT\n")
        self.assertNotIn("PRIVATE_RESOURCE_CONTENT", log.getvalue())
        self.assertIn("error[E0308]: custom diagnostic payload withheld", log.getvalue())

    def test_codeless_build_failures_keep_category_not_payload(self):
        for message, expected in (
            ('error: linking with `cc` failed: exit status: 1 PRIVATE_LINK_ARGS',
             'linker failure; command/output withheld; exit status=1'),
            ('error: failed to run custom build command for `PRIVATE_PACKAGE` PRIVATE_OUTPUT',
             'custom build command failure; command/output withheld'),
        ):
            with contextlib.redirect_stdout(io.StringIO()) as log:
                ignored_tests.compile_diagnostics(message)
            self.assertIn(expected, log.getvalue())
            self.assertNotIn('PRIVATE', log.getvalue())

    def test_standard_panic_details_are_preserved(self):
        for reason in ("index out of bounds: the len is 3 but the index is 4",
                       "attempt to divide by zero", "attempt to add with overflow"):
            output = target(1, "test broken ... FAILED\n"
                            "thread 'broken' panicked at src/lib.rs:12:3:\n" + reason + "\n",
                            summary(failed=1))
            _, log = IgnoredTests().run_output(output, 101)
            self.assertIn("index out of bounds" if reason.startswith("index") else reason, log)
            self.assertNotIn('len=3', log)
            self.assertNotIn('index=4', log)

    def test_source_comes_from_head_not_private_worktree(self):
        with tempfile.TemporaryDirectory() as folder:
            old = os.getcwd()
            try:
                os.chdir(folder)
                subprocess.run(["git", "init", "-q"], check=True)
                path = Path("crates/example/src/lib.rs")
                path.parent.mkdir(parents=True)
                path.write_text('panic!("checked-in reason");\n')
                Path("crates/link.rs").symlink_to("/tmp/private.rs")
                subprocess.run(["git", "add", str(path), "crates/link.rs"], check=True)
                subprocess.run(["git", "-c", "user.name=Test", "-c", "user.email=test@example.com", "commit", "-qm", "fixture"], check=True)
                path.write_text("PRIVATE_WORKTREE")
                Path("crates/private.rs").write_text("PRIVATE_UNTRACKED")
                ignored_tests.source_lines.cache_clear()
                with contextlib.redirect_stdout(io.StringIO()) as log:
                    ignored_tests.panic_source(str(path) + ":1:1")
                    ignored_tests.panic_source("crates/private.rs:1:1")
                    ignored_tests.panic_source("crates/link.rs:1:1")
                    for invalid in ("/tmp/private.rs", "crates/../private.rs", "target/private.rs", ".local/private.rs"):
                        ignored_tests.panic_source(invalid + ":1:1")
                self.assertIn("checked-in reason", log.getvalue())
                self.assertNotIn("PRIVATE", log.getvalue())
                self.assertEqual(log.getvalue().count("source unavailable"), 6)
            finally:
                os.chdir(old)
                ignored_tests.source_lines.cache_clear()

    def test_invalid_source_never_invokes_git(self):
        ignored_tests.source_lines.cache_clear()
        with patch.object(ignored_tests.subprocess, "run") as run:
            for path in ("/etc/private.rs", "crates/../../private.rs", "crates/x/target/private.rs", "crates/x/.local/private.rs"):
                self.assertIsNone(ignored_tests.source_lines(path))
            run.assert_not_called()

    def test_numeric_boolean_and_private_operands(self):
        for left, right in (("1e-10", "-2.5e+30"), ("NaN", "inf"),
                            ("-inf", "0.0"), ("96", "93"), ("true", "false"),
                            ('"PRIVATE"', '[PRIVATE]')):
            output = target(1, "test broken ... FAILED\n"
                            "thread 'broken' panicked at src/lib.rs:12:3:\n"
                            f"assertion `left == right` failed\n left: {left}\n right: {right}\n",
                            summary(failed=1))
            _, log = IgnoredTests().run_output(output, 101)
            self.assertIn('assertion equality failed; payload withheld', log)
            self.assertNotIn('assertion left:', log)
            self.assertNotIn('assertion right:', log)
            self.assertNotIn(' left: ', log)
            self.assertNotIn(' right: ', log)
            self.assertNotIn("PRIVATE", log)

    def test_list_failure_preserves_compiler_headings_not_source(self):
        process = subprocess.CompletedProcess([], 101, "", "error[E0308]: mismatched types\n"
                    " --> src/lib.rs:12:3\nerror: expected `SECRET`, found `OTHER`\n"
                    " PRIVATE_RESOURCE\n")
        with patch.object(ignored_tests.subprocess, "run", return_value=process) as run, contextlib.redirect_stdout(io.StringIO()) as log:
            self.assertIsNone(ignored_tests.run("example", [], 1, 2))
        self.assertEqual(run.call_count, 1)
        self.assertIn("mismatched types", log.getvalue())
        self.assertIn("src/lib.rs:12:3", log.getvalue())
        self.assertNotIn("SECRET", log.getvalue())
        self.assertNotIn("PRIVATE_RESOURCE", log.getvalue())

    def test_bounded_and_unrecognized_diagnostics(self):
        with contextlib.redirect_stdout(io.StringIO()) as log:
            ignored_tests.compile_diagnostics("\n".join(["error: " + "x" * 400] * 30))
            ignored_tests.compile_diagnostics("private unexpected payload")
        self.assertIn("truncated", log.getvalue())
        self.assertIn("no recognized compiler diagnostic", log.getvalue())
        self.assertLess(len(log.getvalue()), 7500)
        self.assertNotIn("private unexpected payload", log.getvalue())

    def test_data_end_memory_telemetry_is_bounded_best_effort(self):
        steps = yaml.safe_load((ROOT / '.github/workflows/ignored-tests.yml').read_text())["jobs"]["ignored-tests-shards"]["steps"]
        step = next(s for s in steps if s.get("name") == "Record data-leg end and compiler cache")
        self.assertEqual(step["if"], "always()")
        script = step["run"]
        self.assertIn("/sys/fs/cgroup/$metric", script)
        self.assertIn("memory.peak memory.max memory.swap.peak memory.swap.max", script)
        self.assertNotIn("cargo", script)
        with tempfile.TemporaryDirectory() as folder:
            Path(folder, "memory.peak").write_text("123456\n")
            Path(folder, "memory.max").write_text("max\n")
            Path(folder, "memory.swap.max").write_text("PRIVATE_PAYLOAD\n")
            # Exercise the actual shell under errexit; absent/invalid files cannot fail it.
            script = script.replace("/sys/fs/cgroup/", folder + "/")
            script = script.replace("sccache --show-stats", "false")
            result = subprocess.run(["bash", "-e", "-c", script], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("memory_peak_bytes=123456", result.stdout)
        self.assertIn("memory_max_bytes=max", result.stdout)
        self.assertIn("memory_swap_peak_bytes=unavailable", result.stdout)
        self.assertIn("memory_swap_max_bytes=unavailable", result.stdout)
        self.assertIn("whole_job_cgroup_including_compilation_tests_file_cache", result.stdout)
        self.assertNotIn("PRIVATE_PAYLOAD", result.stdout)
        self.assertEqual(result.stderr, "")

