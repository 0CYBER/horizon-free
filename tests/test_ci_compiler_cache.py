"""The compiler wrapper preserves routing and falls back only on cache outages."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
WRAPPER = ROOT / "tools/ci/compiler-cache.sh"


@unittest.skipIf(os.name == "nt", "executes a shell wrapper and fake compilers through their shebang lines")
class CompilerCacheTests(unittest.TestCase):
    def test_compiler_routing_preserves_arguments_environment_and_exit_status(self):
        with tempfile.TemporaryDirectory() as directory:
            bin_dir = Path(directory)
            # Fake only the executables: test the actual shell wrapper's routing.
            program = (f"#!{sys.executable}\n"
                       "import json, os, sys\n"
                       "print(json.dumps([sys.argv, os.getenv('CARGO_INCREMENTAL')]))\n"
                       "sys.exit(7)\n")
            for name in ["clippy-driver", "sccache"]:
                path = bin_dir / name
                path.write_text(program)
                path.chmod(0o755)
            env = dict(os.environ, PATH=directory + os.pathsep + os.environ["PATH"],
                       CARGO_INCREMENTAL="1")
            for compiler, args, cached in [
                (str(bin_dir / "clippy-driver"), ["/toolchain/rustc", "-vV"], False),
                (str(bin_dir / "clippy-driver"),
                 ["/toolchain/rustc", "--crate-name", "workspace", "--cfg", 'feature="with spaces"'], False),
                (str(bin_dir / "clippy-driver"),
                 ["/toolchain/rustc", "--crate-name", "mod_points_extra", "-C", "incremental=state"], False),
                ("/toolchain/rustc", ["--crate-name", "dependency", "--emit=metadata"], True),
                ("/toolchain/rustc", ["-vV"], True),
            ]:
                with self.subTest(compiler=compiler, args=args):
                    result = subprocess.run([WRAPPER, compiler, *args], env=env,
                                            text=True, capture_output=True, timeout=5)
                    self.assertEqual(result.returncode, 7, result.stderr)
                    argv, incremental = json.loads(result.stdout)
                    expected = [str(bin_dir / "sccache"), compiler, *args] if cached else [compiler, *args]
                    self.assertEqual(argv, expected)
                    self.assertEqual(incremental, None if cached else "1")

    def test_generated_points_use_cacheable_rustc_and_still_deny_warnings(self):
        with tempfile.TemporaryDirectory() as directory:
            cache = Path(directory) / "sccache"
            cache.write_text(f"#!{sys.executable}\n"
                             "import json, os, sys\n"
                             "print(json.dumps([sys.argv[1:], os.getenv('CARGO_INCREMENTAL')]))\n"
                             "sys.exit(7)\n")
            cache.chmod(0o755)
            env = dict(os.environ, PATH=directory + os.pathsep + os.environ["PATH"],
                       CARGO_INCREMENTAL="1")
            for incremental in [["-C", "incremental=state with spaces"],
                                ["-Cincremental=state with spaces"], []]:
                with self.subTest(incremental=incremental):
                    result = subprocess.run(
                        [WRAPPER, "/toolchain/clippy-driver", "/toolchain/rustc",
                         "--crate-name", "mod_points", *incremental,
                         "-C", "debuginfo=1", "--cfg", 'feature="wasm-host"'],
                        env=env, text=True, capture_output=True, timeout=5,
                    )
                    self.assertEqual(result.returncode, 7, result.stderr)
                    argv, incremental_env = json.loads(result.stdout)
                    self.assertEqual(argv, ["/toolchain/rustc", "--crate-name", "mod_points",
                                            "-C", "debuginfo=1", "--cfg", 'feature="wasm-host"',
                                            "-D", "warnings"])
                    self.assertIsNone(incremental_env)

    def test_cache_errors_retry_then_fall_back_but_compiler_errors_do_not(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            cache = root / "sccache"
            cache.write_text(f"#!{sys.executable}\n"
                             "import os, pathlib, sys\n"
                             "with open(os.environ['CALLS'], 'a') as f: f.write('cache\\n')\n"
                             "if os.environ['MODE'] == 'outage':\n"
                             "    print('sccache: error: Connection to server timed out', file=sys.stderr)\n"
                             "    sys.exit(2)\n"
                             "os.execv(sys.argv[1], sys.argv[1:])\n")
            compiler = root / "rustc"
            compiler.write_text(f"#!{sys.executable}\n"
                                "import os, sys\n"
                                "with open(os.environ['CALLS'], 'a') as f: f.write('compiler\\n')\n"
                                "if os.environ['MODE'] == 'error':\n"
                                "    print('error[E0425]: cannot find function', file=sys.stderr)\n"
                                "    sys.exit(2)\n"
                                "print('compiled')\n")
            cache.chmod(0o755)
            compiler.chmod(0o755)
            for mode, code, expected in [("outage", 0, ["cache", "cache", "compiler"]),
                                          ("error", 2, ["cache", "compiler"]),
                                          ("healthy", 0, ["cache", "compiler"])]:
                calls = root / mode
                env = dict(os.environ, PATH=directory + os.pathsep + os.environ["PATH"],
                           MODE=mode, CALLS=str(calls))
                result = subprocess.run([WRAPPER, compiler], env=env, capture_output=True, text=True, timeout=5)
                self.assertEqual(result.returncode, code, result.stderr)
                self.assertEqual(calls.read_text().splitlines(), expected)
                self.assertEqual('::warning::' in result.stderr, mode == 'outage')
                if mode == 'error':
                    self.assertIn('error[E0425]', result.stderr)


if __name__ == "__main__":
    unittest.main()
