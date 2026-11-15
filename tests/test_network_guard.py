"""The verify guard refuses inet operations even when tests swallow their errors."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools/ci"))
import network_guard
import run_test_binaries


@unittest.skipUnless(sys.platform == "linux", "Linux preload guard")
class NetworkGuard(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.scratch = tempfile.TemporaryDirectory()
        cls.shim = Path(cls.scratch.name, "guard.so")
        source = Path(__file__).resolve().parents[1] / "tools/ci/network_guard.c"
        subprocess.run(["cc", "-shared", "-fPIC", "-Wall", "-Wextra", "-Werror", str(source), "-o", str(cls.shim)], check=True)

        cls.probe = Path(cls.scratch.name, "tests-probe")
        source = Path(cls.scratch.name, "probe.rs")
        source.write_text('#[test] fn green() { assert_eq!(2 + 2, 4); }\n'
                          '#[test] fn swallowed() { let _ = std::net::TcpListener::bind("127.0.0.1:0"); }\n')
        subprocess.run(["rustc", "--edition", "2024", "--test", str(source), "-o", str(cls.probe)], check=True)

    @classmethod
    def tearDownClass(cls):
        cls.scratch.cleanup()

    def invoke(self, source, shim=None):
        binary = {"executable": sys.executable, "cwd": self.scratch.name, "env": {}, "libtest": False,
                  "label": "package target " + "long_test_name_" * 8}
        result = network_guard.run((binary, ["-c", source]), {}, False, shim or self.shim,
                                   run_test_binaries.run_one)
        self.addCleanup(result["output"].close)
        result["output"].seek(0)
        return result, result["output"].read()

    def test_swallowed_socket_error_still_fails_with_full_name(self):
        result, output = self.invoke('import socket\ntry: socket.socket()\nexcept OSError: pass')
        self.assertIn("socket", result["status"])
        self.assertIn("long_test_name_" * 8, output)
        self.assertIn('reason network', output)

    def test_name_resolution_is_forbidden(self):
        result, _ = self.invoke('import socket\ntry: socket.getaddrinfo("localhost", 80)\nexcept OSError: pass')
        self.assertIn("getaddrinfo", result["status"])

    def test_other_libc_resolvers_fail_even_when_the_result_is_swallowed(self):
        calls = {
            "gethostbyname": 'lib.gethostbyname(b"localhost")',
            "gethostbyname2": 'lib.gethostbyname2(b"localhost", 2)',
            "gethostbyname_r": 'lib.gethostbyname_r(b"localhost", host, buf, 1024, ctypes.byref(result), ctypes.byref(error))',
            "gethostbyname2_r": 'lib.gethostbyname2_r(b"localhost", 2, host, buf, 1024, ctypes.byref(result), ctypes.byref(error))',
            "gethostbyaddr": 'lib.gethostbyaddr(ip, 4, 2)',
            "gethostbyaddr_r": 'lib.gethostbyaddr_r(ip, 4, 2, host, buf, 1024, ctypes.byref(result), ctypes.byref(error))',
            "getnameinfo": 'lib.getnameinfo(address, 16, buf, 1024, None, 0, 0)',
        }
        setup = ('import ctypes; lib = ctypes.CDLL(None); '
                 'host = ctypes.create_string_buffer(256); buf = ctypes.create_string_buffer(1024); '
                 'result = ctypes.c_void_p(); error = ctypes.c_int(); '
                 'ip = ctypes.create_string_buffer(bytes([127, 0, 0, 1])); '
                 'address = ctypes.create_string_buffer(bytes([2, 0, 0, 0, 127, 0, 0, 1]) + bytes(8)); ')
        for operation, call in calls.items():
            with self.subTest(operation=operation):
                result, _ = self.invoke(setup + call)
                self.assertEqual(result["status"], f"network guard: {operation}")

    def test_a_helper_loaded_marker_does_not_certify_the_scheduled_process(self):
        def helper_only(binary, env, args, timed, *, listing_env=None):
            # The scheduled target is unguarded, but its dynamic helper loads the library.
            subprocess.run([sys.executable, "-c", "pass"], env={**os.environ, **env}, check=True)
            return {"pid": os.getpid(), "status": "ok", "output": tempfile.TemporaryFile("w+")}
        result = network_guard.run(({"label": "static target"}, []), {}, False, self.shim, helper_only)
        self.addCleanup(result["output"].close)
        self.assertIn("did not load", result["status"])

    def test_green_binary_does_not_isolate(self):
        binary = {"executable": str(self.probe), "cwd": self.scratch.name, "env": {}, "label": "probe"}
        def unexpected_listing(*args):
            self.fail("a green binary must not be isolated")
        result = network_guard.run_binary(binary, {}, ["--exact", "green"], False, self.shim,
                                          run_test_binaries.run_one, unexpected_listing)
        self.addCleanup(result["output"].close)
        self.assertEqual(result["status"], "ok")
        self.assertNotIn("network_tests", result)

    def test_binary_violation_reruns_and_names_the_swallowed_refusal(self):
        binary = {"executable": str(self.probe), "cwd": self.scratch.name, "env": {}, "label": "probe"}
        result = network_guard.run_binary(binary, {}, [], False, self.shim, run_test_binaries.run_one,
                                          run_test_binaries.listed_tests)
        self.addCleanup(result["output"].close)
        self.assertEqual(result["network_tests"], ["swallowed"])
        self.assertNotEqual(result["status"], "ok")
        result["output"].seek(0)
        self.assertIn("probe: swallowed", result["output"].read())

    def test_unreproduced_violation_still_fails_the_binary(self):
        binary = {"executable": sys.executable, "cwd": self.scratch.name, "env": {}, "label": "shared-state"}
        result = network_guard.run_binary(binary, {}, ["-c", "import socket; socket.socket()"], False,
                                          self.shim, run_test_binaries.run_one, lambda *args: [])
        self.addCleanup(result["output"].close)
        result["output"].seek(0)
        self.assertIn("no isolated culprit", result["output"].read())
        self.assertNotEqual(result["status"], "ok")

    def test_unix_socket_is_allowed(self):
        result, _ = self.invoke('import socket; socket.socket(socket.AF_UNIX).close()')
        self.assertEqual(result["status"], "ok")

    def test_children_inherit_guard(self):
        result, _ = self.invoke('import subprocess, sys; subprocess.run([sys.executable, "-c", '
                                '"import socket; socket.socket()"], capture_output=True)')
        self.assertIn("socket", result["status"])

    def test_missing_library_is_not_a_pass(self):
        result, _ = self.invoke('pass', Path(self.scratch.name, "absent.so"))
        self.assertIn("did not load", result["status"])

    def test_invalid_channel_exits_instead_of_losing_violation(self):
        env = {**os.environ, "LD_PRELOAD": str(self.shim), "HORIZON_NETWORK_GUARD_LOG": "/nonexistent/guard/log"}
        result = subprocess.run([sys.executable, "-c", "pass"], env=env)
        self.assertEqual(result.returncode, 125)

    def test_isolated_selection_omits_ignored_tests(self):
        binary = {"label": "package target", "env": {}}
        def listing(binary, env, args):
            return ["ignored"] if "--ignored" in args else ["ignored", "full::test"]
        tasks = list(network_guard.invocations([binary], {}, [], listing))
        self.assertEqual(len(tasks), 1)
        self.assertEqual(tasks[0][1], ["--exact", "full::test"])
        self.assertEqual(tasks[0][0]["isolated_test"], "full::test")

    def test_isolated_selection_removes_filters_but_keeps_option_values(self):
        binary = {"label": "package target", "env": {}}
        args = ["alpha", "beta", "--exact", "--test-threads", "2", "-Z", "unstable-options", "--skip=gamma"]
        def listing(binary, env, selected):
            self.assertEqual(selected[:len(args)], args)
            return [] if "--ignored" in selected else ["alpha", "beta"]
        tasks = list(network_guard.invocations([binary], {}, args, listing))
        options = ["--test-threads", "2", "-Z", "unstable-options", "--skip=gamma"]
        self.assertEqual([task[1] for task in tasks], [options + ["--exact", "alpha"],
                                                      options + ["--exact", "beta"]])

    def test_an_extra_option_separator_is_refused_before_listing(self):
        with self.assertRaises(network_guard.ListingFailed):
            list(network_guard.invocations([], {}, ["--", "alpha"], lambda *args: None))

    def test_unreadable_listing_fails_closed(self):
        with self.assertRaises(network_guard.ListingFailed):
            list(network_guard.invocations([{"label": "target", "env": {}}], {}, [], lambda *args: None))
