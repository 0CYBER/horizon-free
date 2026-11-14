"""The parallel test-binary runner lists Cargo's test executables and reports every failure."""
import contextlib
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
import unittest.mock

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools/ci"))
import run_test_binaries as runner


def artifact(executable, test=True, name="horizon-world", kind="lib"):
    return json.dumps({
        "reason": "compiler-artifact", "package_id": f"path+file:///repo/crates/{name}#0.1.0",
        "manifest_path": f"/repo/crates/{name}/Cargo.toml", "profile": {"test": test},
        "target": {"name": name, "kind": [kind], "src_path": f"/repo/crates/{name}/src/lib.rs"},
        "executable": executable,
    })


class RunTestBinariesTests(unittest.TestCase):
    def test_package_identity_reads_both_id_forms(self):
        self.assertEqual(runner.package_identity("horizon-world 0.1.0 (path+file:///repo/crates/horizon-world)"),
                         ("horizon-world", "0.1.0"))
        self.assertEqual(runner.package_identity("path+file:///repo/crates/horizon-world#0.1.0"),
                         ("horizon-world", "0.1.0"))
        self.assertEqual(runner.package_identity("path+file:///repo/docs/modding/examples#horizon-modding-examples@0.0.0"),
                         ("horizon-modding-examples", "0.0.0"))

    def test_only_test_executables_each_once_with_their_package_environment(self):
        messages = "\n".join([
            artifact("/t/a"), artifact("/t/a"), artifact("/t/b", test=False), artifact(None),
            json.dumps({"reason": "build-finished", "success": True}), "not json",
        ])
        binaries = runner.test_binaries(messages)
        self.assertEqual([b["executable"] for b in binaries], ["/t/a"])
        self.assertEqual(binaries[0]["cwd"], "/repo/crates/horizon-world")
        self.assertEqual(binaries[0]["env"]["CARGO_PKG_NAME"], "horizon-world")
        self.assertEqual(binaries[0]["env"]["CARGO_CRATE_NAME"], "horizon_world")

    def fake_toolchain(self, root: Path, binaries: list[Path]) -> Path:
        """A cargo that lists `binaries` (and records its arguments) and a rustc beside it."""
        listing = [artifact(str(binary)).replace("/repo", str(root)) for binary in binaries]
        cargo = root / "cargo"
        cargo.write_text(f'#!/bin/sh\necho "$@" > {root}/cargo-args\ncat <<\'EOF\'\n' + "\n".join(listing) + "\nEOF\n")
        rustc = root / "rustc"
        rustc.write_text(f'#!/bin/sh\ncase "$1" in --print) echo {root}/sysroot ;; *) echo "host: x86_64-unknown-linux-gnu" ;; esac\n')
        for tool in (cargo, rustc):
            tool.chmod(0o755)
        return cargo

    def run_main(self, *argv: str) -> tuple[int, str]:
        with contextlib.redirect_stdout(io.StringIO()) as log:
            code = runner.main(list(argv))
        return code, log.getvalue()

    @unittest.skipUnless(os.name == "posix" and shutil.which("sh"), "requires a POSIX shell")
    def test_every_binary_runs_and_each_failure_is_listed_at_the_end(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            package = root / "crates/horizon-world"
            package.mkdir(parents=True)
            passing, failing, unstartable = root / "passing", root / "failing", root / "unstartable"
            passing.write_text('#!/bin/sh\necho "cwd=$PWD pkg=$CARGO_PKG_NAME args=$*"\n')
            failing.write_text("#!/bin/sh\necho 'test a::b ... ok'\necho 'test a::c ... FAILED'\nexit 101\n")
            unstartable.write_text("#!/bin/sh\nexit 0\n")  # Not executable: the binary cannot start.
            for binary in (passing, failing):
                binary.chmod(0o755)
            cargo = self.fake_toolchain(root, [unstartable, passing, failing])
            code, output = self.run_main("--jobs", "2", "--", str(cargo), "test", "--workspace", "--", "--ignored")
            self.assertEqual(code, 1)
            # Arguments after cargo's own -- go to every binary, not to the listing.
            self.assertEqual((root / "cargo-args").read_text().split(),
                             ["test", "--workspace", "--no-run", "--message-format=json-render-diagnostics"])
            self.assertIn(f"cwd={package} pkg=horizon-world args=--ignored", output)
            self.assertIn("ran 3 test binaries", output)
            # std's directory beside the toolchain: a proc-macro crate's test binary links it dynamically.
            self.assertIn(f"{root}/sysroot/lib/rustlib/x86_64-unknown-linux-gnu/lib", output)
            summary = output[output.index("FAILED TESTS"):].splitlines()
            self.assertIn("::error::horizon-world lib.rs (failing): exit 101", summary)
            self.assertIn("    a::c", summary)
            self.assertNotIn("    a::b", summary)
            self.assertTrue(any(line.startswith("::error::horizon-world lib.rs (unstartable): could not start")
                                for line in summary), summary)
            self.assertFalse(any("(passing)" in line for line in summary))

    def test_report_names_failed_tests_with_or_without_a_time(self):
        output = tempfile.TemporaryFile("w+")
        output.write("test a::fast ... ok <0.012s>\ntest a::bad ... FAILED <0.100s>\ntest a::plain ... FAILED\n"
                     "child output test a::x ... FAILED\n")
        result = {"binary": {"label": "pkg"}, "output": output, "seconds": 1.0, "max_rss_kib": 0}
        with contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(runner.report(result), ["a::bad", "a::plain"])

    def test_the_log_gives_each_tests_status_and_time(self):
        # The records libtest writes (an ignored test's message follows `ignored:` and may hold spaces and colons).
        self.assertEqual(runner.read_log(
            "ok a::fast <0.012s>\nfailed a::bad <6.250s>\nignored a::skip\nignored: daily a::moved\n"
            "ignored: measurement: cargo test -p x --release y -- --ignored a::measured\nok a::nested\n"
            "ok a::panics_as_expected <0.000s>\n"), {
            "a::fast": ("ok", 0.012), "a::bad": ("failed", 6.25), "a::skip": ("ignored", None),
            "a::moved": ("ignored", None), "a::measured": ("ignored", None), "a::nested": ("ok", None),
            "a::panics_as_expected": ("ok", 0.0)})

    def test_a_passing_binary_without_a_timed_record_for_a_listed_test_is_caught(self):
        result = {"status": "ok", "listed": ["a::timed", "a::ignored", "a::lost", "a::untimed"],
                  "log": {"a::timed": ("ok", 0.1), "a::ignored": ("ignored", None), "a::untimed": ("ok", None)}}
        self.assertEqual(runner.untimed(result), ["a::lost", "a::untimed"])
        self.assertEqual(runner.untimed({**result, "status": "exit 101"}), [])
        self.assertEqual(runner.untimed({**result, "listed": None}), ["(its tests could not be listed)"])

    def test_a_harness_false_target_is_run_untimed(self):
        with tempfile.TemporaryDirectory() as directory:
            manifest = Path(directory) / "Cargo.toml"
            manifest.write_text('[package]\nname = "p"\n[[test]]\nname = "probe"\nharness = false\n[[test]]\nname = "plain"\n')
            self.assertFalse(runner.uses_libtest(str(manifest), {"name": "probe", "kind": ["test"]}))
            self.assertTrue(runner.uses_libtest(str(manifest), {"name": "plain", "kind": ["test"]}))
            self.assertTrue(runner.uses_libtest(str(manifest), {"name": "p", "kind": ["lib"]}))
            binary = Path(directory) / "probe"
            binary.write_text("#!/bin/sh\necho \"args=$*\"\n")
            binary.chmod(0o755)
            result = runner.run_one({"executable": str(binary), "cwd": directory, "env": {}, "libtest": False},
                                    {}, ["-Z", "unstable-options", "--report-time"], timed=True)
            self.assertNotIn("listed", result)
            self.assertEqual(runner.untimed(result), [])

    def test_a_listing_counts_only_when_it_is_the_plain_form_with_its_own_total(self):
        binary = {"executable": "x", "cwd": "."}
        for stdout, expected in [
            ("a::b: test\nc::d: test\n\n2 tests, 0 benchmarks\n", ["a::b", "c::d"]),
            ("\n0 tests, 0 benchmarks\n", []),
            ("a::b: test\nbench::x: bench\n\n1 test, 1 benchmark\n", ["a::b"]),
            ('{ "type": "suite", "event": "discovery" }\n', None),
            ("a::b: test\n\n2 tests, 0 benchmarks\n", None),
        ]:
            with unittest.mock.patch.object(runner.subprocess, "run",
                                            return_value=subprocess.CompletedProcess([], 0, stdout, "")):
                self.assertEqual(runner.listed_tests(binary, {}, []), expected, stdout)

    def test_a_test_over_the_limit_fails_by_name_and_one_over_the_target_warns(self):
        errors, warnings = runner.timing_verdict(
            [(0.5, "a::fast", "pkg"), (5.0, "a::at_the_line", "pkg"), (5.01, "a::over", "pkg"), (2.2, "a::slowish", "pkg")],
            2.0, 5.0)
        self.assertEqual(len(errors), 1)
        self.assertTrue(errors[0].startswith("pkg: a::over took 5.01s"))
        self.assertIn('#[ignore = "daily"]', errors[0])
        self.assertEqual([warning.split(" took")[0] for warning in warnings], ["pkg: a::at_the_line", "pkg: a::slowish"])

    # A libtest stand-in: `--list` lists a::slow, a::fast and a::lost; a run writes its log to the --logfile path,
    # with no record for a::lost when LOSE is set, and prints garbled interleaved output on stdout.
    FAKE_LIBTEST = """#!/bin/sh
for arg in "$@"; do case "$arg" in --list) printf 'a::slow: test\\na::fast: test\\na::lost: test\\n\\n3 tests, 0 benchmarks\\n'; exit 0;; esac; done
log=""; previous=""
for arg in "$@"; do [ "$previous" = "--logfile" ] && log="$arg"; previous="$arg"; done
echo "args=$* bootstrap=$RUSTC_BOOTSTRAP"
echo 'child says hi test a::slow ... okok'
[ -n "$log" ] || exit 0
case " $* " in *' --exact '*)
  [ "$NO_RETRY_RECORD" = 1 ] || printf 'ok a::slow <%ss>\\n' "${RETRY_SECONDS:-7.500}" > "$log"
  exit "${RETRY_EXIT:-0}";;
esac
printf 'ok a::slow <7.500s>\\nok a::fast <0.001s>\\n' > "$log"
[ -n "$LOSE" ] || echo 'ok a::lost <0.002s>' >> "$log"
"""

    @unittest.skipUnless(os.name == "posix" and shutil.which("sh"), "requires a POSIX shell")
    def test_time_limits_time_every_test_from_the_log_and_fail_closed(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "crates/horizon-world").mkdir(parents=True)
            binary = root / "timed"
            binary.write_text(self.FAKE_LIBTEST)
            binary.chmod(0o755)
            cargo = self.fake_toolchain(root, [binary])
            code, output = self.run_main("--jobs", "1", "--time-limits", "2,5", "--", str(cargo), "test")
            self.assertEqual(code, 1)
            self.assertIn("args=-Z unstable-options --report-time --logfile ", output)
            self.assertIn("bootstrap=1", output)
            self.assertIn("::error::horizon-world lib.rs (timed): a::slow took 7.50s", output)
            self.assertIn("3 tests timed, 7.5 test-seconds in total; slowest: a::slow 7.50s", output)
            self.assertNotIn("no timed record", output)
            with unittest.mock.patch.dict(os.environ, {"LOSE": "1"}):
                code, output = self.run_main("--jobs", "1", "--time-limits", "2,9", "--", str(cargo), "test")
            self.assertEqual(code, 1)
            self.assertIn("::error::horizon-world lib.rs (timed): 1 listed tests have no timed record in libtest's "
                          "log, so the guard cannot judge them: a::lost", output)
            # Without the option nothing is added and the times are not judged.
            code, output = self.run_main("--jobs", "1", "--", str(cargo), "test")
            self.assertEqual(code, 0)
            self.assertIn("args= bootstrap=\n", output)

    @unittest.skipUnless(os.name == "posix" and shutil.which("sh"), "requires a POSIX shell")
    def test_only_an_isolated_passing_measurement_can_clear_a_hard_limit(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "crates/horizon-world").mkdir(parents=True)
            binary = root / "timed"
            binary.write_text(self.FAKE_LIBTEST)
            binary.chmod(0o755)
            cargo = self.fake_toolchain(root, [binary])
            for env, expected, measured in [
                ({"RETRY_SECONDS": "0.125"}, 0, "0.12s"),
                ({"RETRY_SECONDS": "5.001"}, 1, "5.00s"),
                ({"RETRY_SECONDS": "0.125", "RETRY_EXIT": "101"}, 1, "0.12s"),
                ({"NO_RETRY_RECORD": "1"}, 1, "unavailable"),
            ]:
                with self.subTest(env=env), unittest.mock.patch.dict(os.environ, env):
                    code, output = self.run_main("--jobs", "2", "--time-limits", "2,5", "--", str(cargo), "test")
                    self.assertEqual(code, expected, output)
                    self.assertIn("--exact a::slow --test-threads=1 --logfile", output)
                    self.assertEqual(output.count("timing confirmation:"), 1)
                    self.assertIn(f"first 7.50s, isolated {measured}", output)

    def test_a_guarded_run_confirms_a_slow_test_under_the_guard(self):
        binary = {"label": "horizon-world lib.rs"}
        seen = []

        def guarded(invocation, env, timed, shim, run_one):
            seen.append((invocation, shim, run_one))
            return {"binary": binary, "seconds": 0.1, "max_rss_kib": 0, "status": "network guard: socket",
                    "output": io.StringIO(), "log": {"a::slow": ("ok", 0.1)}}

        with unittest.mock.patch.object(runner.network_guard, "run", guarded), \
                contextlib.redirect_stdout(io.StringIO()):
            errors = runner.confirm_slow_tests([(7.5, "a::slow", binary["label"])], [binary], {}, [], 5.0,
                                               Path("/guard.so"))
        self.assertEqual(len(seen), 1)
        (rerun, args), shim, run_one = seen[0]
        self.assertEqual((rerun, shim, run_one), (binary, Path("/guard.so"), runner.run_one))
        self.assertIn("--exact", args)
        self.assertEqual(len(errors), 1)
        self.assertIn("network guard: socket", errors[0])

    @unittest.skipUnless(os.name == "posix" and shutil.which("sh"), "requires a POSIX shell")
    def test_a_listing_without_test_binaries_fails(self):
        with tempfile.TemporaryDirectory() as directory:
            cargo = self.fake_toolchain(Path(directory), [])
            code, output = self.run_main("--jobs", "2", "--", str(cargo), "test")
            self.assertEqual(code, 1)
            self.assertIn("cargo listed no test binaries", output)


if __name__ == "__main__":
    unittest.main()
