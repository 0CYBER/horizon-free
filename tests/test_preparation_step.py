"""Observable preparation diagnostics without executing Cargo, Java or the preparer."""
import os
import json
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / "tools/ci/preparation_step.py"


class PreparationStepTests(unittest.TestCase):
    def invoke(self, root, code, step="resource-export", **environment):
        return subprocess.run([sys.executable, str(SCRIPT), step, str(root / "private.log"),
                               sys.executable, "-c", code], capture_output=True, text=True,
                              env=dict(os.environ, **environment))

    def diagnostic(self, step, status, reason, last="redacted-unclassified"):
        return (f"::error::preparation step={step} exit={status} reason_hint={reason} "
                "expected_key=unavailable-or-invalid found_key=not-recorded cache_state=unavailable "
                f"last_line_classification={last}\n"
                "unclassified: see private log (not retained on per-job runners)\n")

    def test_failure_is_private_and_status_survives(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            for marker, reason in [("No space left on device", "disk-full"),
                                   ("Permission denied", "permission-denied"),
                                   ("Read-only file system", "read-only-filesystem"),
                                   ("OutOfMemoryError", "java-out-of-memory"),
                                   ("Could not resolve host", "network-dns"),
                                   ("Connection timed out", "network-timeout"),
                                   ("No such file or directory", "missing-file"),
                                   ("unknown failure", "unclassified")]:
                with self.subTest(reason=reason):
                    payload = "token=SECRET .local/minecraft/26.3/file {NBT: excerpt} ::error::injected " + marker
                    result = self.invoke(root, f"import sys; print({payload!r}); sys.exit(23)")
                    self.assertEqual(result.returncode, 23)
                    self.assertEqual(result.stdout, self.diagnostic("resource-export", 23, reason, reason if reason != "unclassified" else "redacted-unclassified"))
                    sidecar = (root / "private.diagnostic.json").read_text()
                    self.assertNotIn("SECRET", sidecar)
                    self.assertNotIn(".local", sidecar)
                    self.assertNotIn("excerpt", sidecar)
                    self.assertEqual(json.loads(sidecar)["schema_version"], 1)
                    self.assertEqual(result.stderr, "")
                    self.assertIn(payload, (root / "private.log").read_text())

    def test_safe_last_line_and_key_validation(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            for key, expected in [("0123456789abcdef", "0123456789abcdef"),
                                  ("0123456789abcdefSECRET", "unavailable-or-invalid"),
                                  (".local/minecraft/secret", "unavailable-or-invalid"),
                                  ("", "unavailable-or-invalid")]:
                result = self.invoke(root, "import sys; print('Permission denied'); sys.exit(2)",
                                     HORIZON_DIAGNOSTIC_EXPECTED_KEY=key,
                                     HORIZON_DIAGNOSTIC_CACHE_STATE="missing-entry-list")
                self.assertEqual(result.returncode, 2)
                self.assertTrue(result.stdout.endswith("\nPermission denied\n"))
                data = json.loads((root / "private.diagnostic.json").read_text())
                self.assertEqual(data["expected_key"], expected)
                self.assertEqual(data["found_key"], "not-recorded")
                self.assertEqual(data["cache_state"], "missing-entry-list")
                self.assertNotIn("SECRET", result.stdout + result.stderr + json.dumps(data))
                self.assertNotIn(".local", result.stdout + result.stderr + json.dumps(data))

    def test_success_never_prints_private_output(self):
        with tempfile.TemporaryDirectory() as folder:
            result = self.invoke(Path(folder), "print('SECRET No space left on device')")
            self.assertEqual((result.returncode, result.stdout, result.stderr), (0, "", ""))

    def test_missing_log_parent_has_safe_failure(self):
        with tempfile.TemporaryDirectory() as folder:
            result = self.invoke(Path(folder) / "absent", "raise AssertionError('must not run')")
            self.assertEqual(result.returncode, 1)
            self.assertEqual(result.stdout, self.diagnostic("resource-export", 1, "missing-file") + "preparation diagnostic sidecar unavailable\n")
            self.assertEqual(result.stderr, "")

    def test_signal_and_step_identity(self):
        with tempfile.TemporaryDirectory() as folder:
            result = self.invoke(Path(folder), "import os, signal; os.kill(os.getpid(), signal.SIGTERM)", "preparer-build")
            self.assertEqual(result.returncode, 143)
            self.assertEqual(result.stdout, self.diagnostic("preparer-build", -15, "unclassified"))

    def test_shell_cleanup_preserves_failure(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root / "tools/ci").mkdir(parents=True)
            for name in ("vanilla-data.sh", "preparation_step.py"):
                shutil.copy(SCRIPT.parent / name, root / "tools/ci" / name)
            (root / "research-lock.json").write_text('{"version": "26.3"}\n')
            (root / "source/minecraft/26.3").mkdir(parents=True)
            (root / "source/minecraft/26.3/prepared.json").write_text('{}')
            (root / "bin").mkdir()
            cargo = root / "bin/cargo"
            cargo.write_text('#!/bin/sh\nexit 0\n')
            cargo.chmod(0o755)
            (root / "target/release").mkdir(parents=True)
            prepare = root / "target/release/horizon-prepare"
            prepare.write_text('#!/bin/sh\nmkdir .local/scratch\necho "SECRET No space left on device"\nexit 23\n')
            prepare.chmod(0o755)
            env = dict(os.environ, HORIZON_PERSISTENT_CACHE="true", HORIZON_PREPARED_CACHE=str(root / "cache"),
                       HORIZON_SHARED_SOURCE=str(root / "source"), HORIZON_PREPARE_KEY="0123456789abcdef",
                       RUNNER_TEMP=str(root), CARGO_TARGET_DIR=str(root / "target"),
                       PATH=str(root / "bin") + os.pathsep + os.environ["PATH"])
            result = subprocess.run(["bash", "tools/ci/vanilla-data.sh"], cwd=root, env=env,
                                    capture_output=True, text=True)
            self.assertEqual(result.returncode, 23, result.stderr)
            self.assertIn('step=resource-export exit=23 reason_hint=disk-full', result.stdout)
            self.assertNotIn('SECRET', result.stdout + result.stderr)
            self.assertFalse((root / '.local/scratch').exists())
            self.assertFalse((root / 'cache/prepared/0123456789abcdef').exists())
            self.assertIn('expected_key=0123456789abcdef found_key=not-recorded cache_state=missing-entry-list', result.stdout)
            stale = root / 'cache/prepared/0123456789abcdef'
            stale.mkdir()
            (stale / '.entries').write_text('other-output\n')
            again = subprocess.run(["bash", "tools/ci/vanilla-data.sh"], cwd=root, env=env,
                                   capture_output=True, text=True)
            self.assertEqual(again.returncode, 23, again.stderr)
            self.assertIn('cache_state=missing-horizon-client', again.stdout)
            self.assertNotIn('SECRET', again.stdout + again.stderr)
            self.assertTrue((root / 'source/minecraft/26.3/prepared.json').exists())

    def test_a_given_preparer_is_run_and_cargo_is_not(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root / "tools/ci").mkdir(parents=True)
            for name in ("vanilla-data.sh", "preparation_step.py"):
                shutil.copy(SCRIPT.parent / name, root / "tools/ci" / name)
            (root / "research-lock.json").write_text('{"version": "26.3"}\n')
            (root / "source/minecraft/26.3").mkdir(parents=True)
            (root / "source/minecraft/26.3/prepared.json").write_text('{}')
            (root / "bin").mkdir()
            cargo = root / "bin/cargo"
            cargo.write_text(f'#!/bin/sh\ntouch "{(root / "cargo-ran").as_posix()}"\nexit 1\n')
            cargo.chmod(0o755)
            (root / "built").mkdir()
            given = root / "built/horizon-prepare"
            given.write_text('#!/bin/sh\necho "the given preparer"\nexit 23\n')
            given.chmod(0o755)
            env = dict(os.environ, HORIZON_PERSISTENT_CACHE="true", HORIZON_PREPARED_CACHE=str(root / "cache"),
                       HORIZON_SHARED_SOURCE=str(root / "source"), HORIZON_PREPARE_KEY="0123456789abcdef",
                       RUNNER_TEMP=str(root), CARGO_TARGET_DIR=str(root / "target"),
                       HORIZON_PREPARE_BINARY=str(given), PATH=str(root / "bin") + os.pathsep + os.environ["PATH"])
            result = subprocess.run(["bash", "tools/ci/vanilla-data.sh"], cwd=root, env=env,
                                    capture_output=True, text=True)
            self.assertEqual(result.returncode, 23, (result.stdout, result.stderr))
            self.assertIn("step=resource-export exit=23", result.stdout)
            self.assertFalse((root / "cargo-ran").exists())
            missing = subprocess.run(["bash", "tools/ci/vanilla-data.sh"], cwd=root, capture_output=True, text=True,
                                     env=dict(env, HORIZON_PREPARE_BINARY=str(root / "built/none")))
            self.assertEqual(missing.returncode, 1, missing.stderr)
            self.assertIn("HORIZON_PREPARE_BINARY is not a program", missing.stdout)
            self.assertFalse((root / "cargo-ran").exists())

    def test_a_kept_preparation_is_reused_only_unchanged_and_each_job_gets_its_own_copy(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root / "tools/ci").mkdir(parents=True)
            for name in ("vanilla-data.sh", "preparation_step.py", "seed_prepared_extras.py"):
                shutil.copy(SCRIPT.parent / name, root / "tools/ci" / name)
            shutil.copy(SCRIPT.parents[1] / "prepare_output.py", root / "tools/prepare_output.py")
            (root / "research-lock.json").write_text('{"version": "26.3"}\n')
            (root / "source/minecraft/26.3").mkdir(parents=True)
            (root / "source/minecraft/26.3/prepared.json").write_text('{}')
            (root / "source/horizon-client/26.3").mkdir(parents=True)
            (root / "bin").mkdir()
            cargo = root / "bin/cargo"
            cargo.write_text('#!/bin/sh\nexit 0\n')
            cargo.chmod(0o755)
            (root / "target/release").mkdir(parents=True)
            prepare = root / "target/release/horizon-prepare"
            prepare.write_text('#!/bin/sh\nmkdir -p .local/horizon-client/26.3\necho one > .local/horizon-client/26.3/a\n'
                               'echo "prepared" >> "$PREPARED_RUNS"\n')
            prepare.chmod(0o755)
            env = dict(os.environ, HORIZON_PERSISTENT_CACHE="true", HORIZON_PREPARED_CACHE=str(root / "cache"),
                       HORIZON_SHARED_SOURCE=str(root / "source"), HORIZON_PREPARE_KEY="0123456789abcdef",
                       RUNNER_TEMP=str(root), CARGO_TARGET_DIR=str(root / "target"),
                       PREPARED_RUNS=str(root / "runs"), PATH=str(root / "bin") + os.pathsep + os.environ["PATH"])

            def run():
                result = subprocess.run(["bash", "tools/ci/vanilla-data.sh"], cwd=root, env=env,
                                        capture_output=True, text=True)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                return result.stdout

            kept = root / "cache/prepared/0123456789abcdef"
            run()
            self.assertEqual((root / "runs").read_text().count("prepared"), 1)
            # The job reads its own copy, never the kept tree.
            self.assertEqual((root / ".local/horizon-client").resolve(), (root / "cache/job/horizon-client").resolve())
            (root / ".local/horizon-client/26.3/written-by-a-test").write_text("x")
            self.assertFalse((kept / "horizon-client/26.3/written-by-a-test").exists())
            self.assertIn("fresh", run())
            self.assertEqual((root / "runs").read_text().count("prepared"), 1)
            self.assertFalse((root / ".local/horizon-client/26.3/written-by-a-test").exists())
            # A kept tree changed behind the script's back (or kept from before manifests) is prepared again.
            (kept / "horizon-client/26.3/extra").write_text("x")
            again = run()
            self.assertIn("does not match its manifest", again)
            self.assertIn("files added 1, removed 0, changed 0", again)
            self.assertEqual((root / "runs").read_text().count("prepared"), 2)
            self.assertFalse((kept / "horizon-client/26.3/extra").exists())
            (kept / ".manifest").unlink()
            (kept / ".manifest-files").unlink()
            self.assertIn("expected none", run())
            self.assertEqual((root / "runs").read_text().count("prepared"), 3)

    def test_bounded_log_does_not_hide_exit(self):
        with tempfile.TemporaryDirectory() as folder:
            result = self.invoke(Path(folder), "import sys; print('x' * 1100000); print('No space left on device SECRET'); sys.exit(7)", "seed-extras")
            self.assertEqual(result.returncode, 7)
            self.assertEqual(result.stdout, self.diagnostic("seed-extras", 7, "unclassified", "disk-full"))


if __name__ == "__main__":
    unittest.main()
