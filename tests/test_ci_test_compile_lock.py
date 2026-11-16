"""Selective compiler locking preserves parallelism and the configured cache wrapper."""
import json
import os
from pathlib import Path
import select
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
WRAPPER = ROOT / "tools/ci/test-compile-lock.sh"


@unittest.skipIf(os.name == "nt", "requires Bash, flock and pipe readiness")
class TestCompileLockTests(unittest.TestCase):
    def test_background_compiler_child_cannot_keep_the_lock_after_compiler_exit(self):
        import signal

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            pid_file = root / "child-pid"
            compiler = root / "fake-rustc"
            compiler.write_text(f"#!{sys.executable}\n"
                                "import os, pathlib, subprocess, sys\n"
                                "if '--spawn-child' in sys.argv:\n"
                                "    child = subprocess.Popen(['sleep', '30'], close_fds=False,\n"
                                "                             stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)\n"
                                "    pathlib.Path(os.environ['CHILD_PID']).write_text(str(child.pid))\n"
                                "sys.exit(7)\n")
            compiler.chmod(0o755)
            env = dict(os.environ, RUNNER_TEMP=directory, CHILD_PID=str(pid_file),
                       HORIZON_TEST_RUSTC_WRAPPER="")
            try:
                first = subprocess.run([WRAPPER, compiler, '--crate-name', 'horizon_client_lib',
                                        '--test', '--spawn-child'], env=env, timeout=5)
                self.assertEqual(first.returncode, 7)
                child_pid = int(pid_file.read_text())
                os.kill(child_pid, 0)  # Descendant is still alive when the second call runs.
                second = subprocess.run([WRAPPER, compiler, '--crate-name', 'horizon_client',
                                         '--test'], env=env, timeout=5)
                self.assertEqual(second.returncode, 7)
                os.kill(child_pid, 0)
            finally:
                if pid_file.exists():
                    os.kill(int(pid_file.read_text()), signal.SIGTERM)

    def test_response_file_and_equals_form_take_the_lock_without_rewriting_arguments(self):
        import fcntl

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            compiler = root / "fake-rustc"
            compiler.write_text(f"#!{sys.executable}\n"
                                "import json, sys\n"
                                "print(json.dumps(sys.argv[1:]), flush=True)\n"
                                "sys.exit(7)\n")
            compiler.chmod(0o755)
            response = root / "arguments with spaces"
            response.write_text('--crate-name\nhorizon_client_lib\n--cfg\nfeature="with spaces"\n--test')
            env = dict(os.environ, RUNNER_TEMP=directory, HORIZON_TEST_RUSTC_WRAPPER="")
            for args in (["@" + str(response)], ["--crate-name=horizon_client", "--test"]):
                with self.subTest(args=args), (root / "horizon-lib-test-compile.lock").open("w") as lock:
                    fcntl.flock(lock, fcntl.LOCK_EX)
                    process = subprocess.Popen([WRAPPER, compiler, *args], env=env,
                                               stdout=subprocess.PIPE, text=True)
                    try:
                        self.assertFalse(select.select([process.stdout], [], [], 0.2)[0],
                                         "compiler bypassed the held test-unit lock")
                        fcntl.flock(lock, fcntl.LOCK_UN)
                        output, _ = process.communicate(timeout=5)
                        self.assertEqual(process.returncode, 7)
                        self.assertEqual(json.loads(output), args)
                    finally:
                        if process.poll() is None:
                            process.kill()
                        process.communicate()

    def test_large_test_units_serialize_while_other_calls_pass_through(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            compiler = root / "fake-rustc"
            compiler.write_text(f"#!{sys.executable}\n"
                                "import json, sys\n"
                                "print(json.dumps(sys.argv[1:]), flush=True)\n"
                                "if '--hold' in sys.argv: sys.stdin.readline()\n"
                                "sys.exit(7)\n")
            compiler.chmod(0o755)
            cache = root / "cache-wrapper"
            cache.write_text('#!/usr/bin/env bash\necho cached >> "$RUNNER_TEMP/cache-calls"\nexec "$@"\n')
            cache.chmod(0o755)
            env = dict(os.environ, RUNNER_TEMP=directory,
                       HORIZON_TEST_RUSTC_WRAPPER=str(cache))
            args = ["--crate-name", "horizon_client_lib", "--test", "--hold"]
            first = subprocess.Popen([WRAPPER, compiler, *args], env=env,
                                     stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
            second = None
            try:
                self.assertTrue(select.select([first.stdout], [], [], 5)[0])
                self.assertEqual(json.loads(first.stdout.readline()), args)
                second_args = ["--test", "--crate-name", "horizon_client", "--cfg", 'feature="with spaces"']
                second = subprocess.Popen([WRAPPER, compiler, *second_args], env=env,
                                          stdout=subprocess.PIPE, text=True)
                self.assertFalse(select.select([second.stdout], [], [], 0.2)[0],
                                 "second large test started before first released its lock")
                for other_args in (["--crate-name", "unrelated", "--test"],
                                   ["--crate-name", "horizon_client"],
                                   ["--crate-name", "horizon_client_lib_extra", "--test"]):
                    result = subprocess.run([WRAPPER, compiler, *other_args], env=env,
                                            capture_output=True, text=True, timeout=5)
                    self.assertEqual(result.returncode, 7)
                    self.assertEqual(json.loads(result.stdout), other_args)
                first.communicate("release\n", timeout=5)
                output, _ = second.communicate(timeout=5)
                self.assertEqual(first.returncode, 7)
                self.assertEqual(second.returncode, 7)
                self.assertEqual(json.loads(output), second_args)
                self.assertEqual((root / "cache-calls").read_text().splitlines(), ["cached"] * 5)
                # An unset upstream wrapper executes the compiler directly too.
                env.pop("HORIZON_TEST_RUSTC_WRAPPER")
                direct = subprocess.run([WRAPPER, compiler, "--version"], env=env,
                                        capture_output=True, text=True, timeout=5)
                self.assertEqual(direct.returncode, 7)
                self.assertEqual(json.loads(direct.stdout), ["--version"])
            finally:
                for process in (first, second):
                    if process is not None:
                        if process.poll() is None:
                            process.kill()
                        process.communicate()
