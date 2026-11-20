"""tools/ci/cargo_isolation.py: no cargo configuration from outside the checkout, a fresh cargo home, no values in
its output."""
import contextlib
import io
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from tools.ci import cargo_isolation

ROOT = Path(__file__).resolve().parents[1]
SECRET = "s3cret-token-value-4711"


def clean_environ(**extra):
    environ = {key: value for key, value in os.environ.items() if not cargo_isolation.forbidden(key)}
    environ.update(extra)
    return environ


class CargoIsolationTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.top = Path(temporary.name).resolve()
        self.checkout = self.top / "work" / "horizon" / "horizon"
        (self.checkout / ".cargo").mkdir(parents=True)
        # The repository's own configuration is allowed.
        (self.checkout / ".cargo" / "config.toml").write_text('[build]\nrustflags = ["-Zthreads=8"]\n')

    def problems(self, environ=None):
        return cargo_isolation.problems(str(self.checkout), clean_environ() if environ is None else environ,
                                        top=str(self.top))

    def run_guard(self, **extra):
        runner_temp = self.top / "runner-temp"
        runner_temp.mkdir(exist_ok=True)
        github_env = runner_temp / "github-env"
        github_env.touch()
        environ = clean_environ(RUNNER_TEMP=str(runner_temp), GITHUB_ENV=str(github_env))
        environ.update(extra)
        result = subprocess.run([sys.executable, str(ROOT / "tools/ci/cargo_isolation.py"), str(self.checkout),
                                 "--top", str(self.top)], env=environ, capture_output=True, text=True)
        return result, runner_temp / "cargo-home", github_env.read_text()

    def test_a_clean_tree_gets_a_fresh_cargo_home(self):
        self.assertEqual(self.problems(), [])
        runner_temp = self.top / "runner-temp"
        (runner_temp / "cargo-home" / "registry").mkdir(parents=True)
        (runner_temp / "cargo-home" / "config.toml").write_text("[build]\n")
        result, home, github_env = self.run_guard()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(list(home.iterdir()), [])
        self.assertEqual(github_env, f"CARGO_HOME={home}\n")
        self.assertEqual(result.stdout, cargo_isolation.CLEAN + "\n")
        # No path from the runner's variables in either stream.
        for text in (str(runner_temp), runner_temp.name, "runner-temp", "cargo-home"):
            self.assertNotIn(text, result.stdout + result.stderr)

    def test_a_failing_ancestor_walk_prints_a_fixed_message_and_no_path(self):
        error = OSError(2, "No such file or directory", "/runner/secret-link-5d2e")
        out, err = io.StringIO(), io.StringIO()
        with mock.patch.object(cargo_isolation.os.path, "realpath", side_effect=error), \
                contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            status = cargo_isolation.main([str(self.checkout), "--top", str(self.top)])
        self.assertEqual(status, 1)
        self.assertEqual(out.getvalue(), "::error::could not read the checkout's parent folders\n")
        self.assertEqual(err.getvalue(), "")
        self.assertNotIn("secret-link-5d2e", out.getvalue())

    def test_setup_failures_print_a_fixed_message_and_no_path(self):
        unopenable = self.top / "missing-env-dir-7f3a" / "github-env"
        result, home, _ = self.run_guard(GITHUB_ENV=str(unopenable))
        output = result.stdout + result.stderr
        self.assertEqual(result.returncode, 1)
        self.assertEqual(result.stdout, "::error::could not write the run's environment file\n")
        self.assertEqual(result.stderr, "")
        self.assertNotIn("missing-env-dir-7f3a", output)
        self.assertNotIn("Traceback", output)
        self.assertNotIn(cargo_isolation.CLEAN, output)
        not_a_folder = self.top / "plain-file-9c1e"
        not_a_folder.write_text("")
        result, _, _ = self.run_guard(RUNNER_TEMP=str(not_a_folder))
        self.assertEqual(result.returncode, 1)
        self.assertEqual(result.stdout + result.stderr, "::error::could not create the run's cargo home\n")

    def test_a_config_at_each_ancestor_fails(self):
        levels = [self.checkout.parent, self.checkout.parent.parent, self.top]
        for level in levels:
            for name in ("config", "config.toml"):
                with self.subTest(level=str(level), name=name):
                    path = level / ".cargo" / name
                    path.parent.mkdir(exist_ok=True)
                    path.write_text('[build]\nrustc-wrapper = "x"\n')
                    try:
                        found = self.problems()
                        self.assertEqual(len(found), 1, found)
                        self.assertIn(str(path), found[0])
                        self.assertIn("(tables: build)", found[0])
                    finally:
                        path.unlink()
        result, home, github_env = self.run_guard()
        self.assertEqual(result.returncode, 0)
        (self.top / ".cargo" / "config.toml").write_text("[net]\noffline = true\n")
        result, _, _ = self.run_guard()
        self.assertEqual(result.returncode, 1)

    def test_a_symlinked_config_fails_even_when_dangling(self):
        (self.top / ".cargo").mkdir()
        link = self.top / ".cargo" / "config.toml"
        try:
            os.symlink(self.top / "elsewhere.toml", link)
        except (OSError, NotImplementedError):
            self.skipTest("no symlinks here")
        found = self.problems()
        self.assertEqual(len(found), 1, found)
        self.assertIn("could not be parsed", found[0])
        (self.top / "elsewhere.toml").write_text('[source.crates-io]\nreplace-with = "mirror"\n')
        self.assertIn("(tables: source)", self.problems()[0])

    def test_each_forbidden_variable_fails_and_the_allow_list_passes(self):
        for name in sorted(cargo_isolation.FORBIDDEN):
            with self.subTest(name=name):
                self.assertEqual(self.problems(clean_environ(**{name: "1"})), [f"variable set: {name}"])
                self.assertEqual(self.problems(clean_environ(**{name: ""})), [])
        for prefix in cargo_isolation.FORBIDDEN_PREFIXES:
            with self.subTest(prefix=prefix):
                self.assertEqual(self.problems(clean_environ(**{prefix + "X": "1", prefix + "Y": "1"})),
                                 [f"variables set: {prefix}* (2)"])
                self.assertEqual(self.problems(clean_environ(**{prefix + "X": ""})), [])
        self.assertEqual(self.problems(clean_environ(CARGO_INCREMENTAL="0", CARGO_HOME="/x", CARGO_TERM_COLOR="always",
                                                     RUSTC_WRAPPER="", RUSTC_WORKSPACE_WRAPPER="")), [])
        self.assertEqual(self.problems(clean_environ(CARGO_INCREMENTAL="1")), ["variable set: CARGO_INCREMENTAL"])

    def test_the_runners_job_count_is_accepted_as_a_plain_positive_integer(self):
        for value in ("1", "6", "10", "9999"):
            with self.subTest(value=value):
                self.assertEqual(self.problems(clean_environ(CARGO_BUILD_JOBS=value)), [])
        result, _, _ = self.run_guard(CARGO_BUILD_JOBS="7385")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        # Named just before the last line, never with its value.
        self.assertEqual(result.stdout, f"allowed: CARGO_BUILD_JOBS\n{cargo_isolation.CLEAN}\n")
        self.assertNotIn("7385", result.stdout + result.stderr)
        # Not when anything else fails the guard.
        result, _, _ = self.run_guard(CARGO_BUILD_JOBS="10", RUSTC_WRAPPER="sccache")
        self.assertEqual(result.returncode, 1)
        self.assertNotIn("allowed:", result.stdout + result.stderr)
        # Anything else, an empty value too (unlike every other variable, where empty means unset).
        for value in ("0", "-1", "default", "", "4 x", "4; x", " 4", "4\n", "04", "10000", "+4", "4.0",
                      "1٢", "١", "1２"):
            with self.subTest(value=value):
                self.assertEqual(self.problems(clean_environ(CARGO_BUILD_JOBS=value)),
                                 ["variable set: CARGO_BUILD_JOBS (not a plain job count)"])
        result, _, _ = self.run_guard(CARGO_BUILD_JOBS="default")
        self.assertEqual(result.returncode, 1)
        self.assertNotIn("allowed:", result.stdout)
        self.assertNotIn("default", result.stdout + result.stderr)
        self.assertNotIn(cargo_isolation.CLEAN, result.stdout)
        # Other empty variables still count as unset.
        self.assertEqual(self.problems(clean_environ(RUSTC_WRAPPER="", CARGO_BUILD_TARGET="")), [])

    def test_only_cargos_documented_family_names_are_printed(self):
        for name in ("CARGO_BUILD_RUSTC_WRAPPER", "CARGO_BUILD_TARGET_DIR", "CARGO_BUILD_RUSTFLAGS",
                     "CARGO_PROFILE_RELEASE_LTO", "CARGO_PROFILE_DEV_OPT_LEVEL", "CARGO_TARGET_DIR"):
            with self.subTest(name=name):
                self.assertEqual(self.problems(clean_environ(**{name: "x"})), [f"variable set: {name}"])
        for name, family in (("CARGO_BUILD_UNKNOWN", "CARGO_BUILD_"), ("CARGO_PROFILE_CUSTOM_LTO", "CARGO_PROFILE_"),
                             ("CARGO_PROFILE_RELEASE_BUILD_OVERRIDE_OPT_LEVEL", "CARGO_PROFILE_"),
                             ("CARGO_TARGET_X86_64_APPLE_DARWIN_LINKER", "CARGO_TARGET_"),
                             ("CARGO_NET_OFFLINE", "CARGO_NET_"), ("CARGO_REGISTRY_TOKEN", "CARGO_REGISTRY_")):
            with self.subTest(name=name):
                found = self.problems(clean_environ(**{name: "x"}))
                self.assertEqual(found, [f"variables set: {family}* (1)"])
                self.assertNotIn(name, found[0])

    def test_no_value_ever_reaches_the_output(self):
        (self.top / ".cargo").mkdir()
        # A secret as a value, as a registry's name, inside a URL used as a [patch] key, and as a top-level name.
        (self.top / ".cargo" / "config.toml").write_text(
            f'[registries.x]\ntoken = "{SECRET}"\n'
            f'[registries.{SECRET}]\nindex = "sparse+https://example.org/"\n'
            f'[build]\nrustc-wrapper = "/{SECRET}/sccache"\n'
            f'[patch."https://user:{SECRET}@example.org/repo"]\nfoo = {{ path = "x" }}\n'
            f'[{SECRET}]\nkey = 1\n')
        (self.checkout.parent / ".cargo").mkdir()
        (self.checkout.parent / ".cargo" / "config").write_text(f'token = "{SECRET}\n')  # unparsable
        # And in variables' values and in the free part of their names.
        in_name = SECRET.upper().replace("-", "_")
        result, home, github_env = self.run_guard(**{
            "CARGO_REGISTRY_TOKEN": SECRET, "RUSTC_WRAPPER": f"/{SECRET}",
            f"CARGO_REGISTRIES_{in_name}_TOKEN": "x", f"CARGO_ALIAS_{in_name}": "build",
            f"CARGO_BUILD_{in_name}": "x", f"CARGO_PROFILE_{in_name}_LTO": "x", "CARGO_BUILD_JOBS": f"4{SECRET}"})
        output = result.stdout + result.stderr
        self.assertEqual(result.returncode, 1)
        self.assertNotIn(cargo_isolation.CLEAN, output)
        self.assertNotIn("s3cret", output.lower())
        self.assertNotIn("4711", output)
        for expected in ("(tables: build, patch, registries, other (1))", "could not be parsed",
                         "variables set: CARGO_REGISTRY_* (1)", "variables set: CARGO_REGISTRIES_* (1)",
                         "variables set: CARGO_ALIAS_* (1)", "variable set: RUSTC_WRAPPER",
                         "variables set: CARGO_BUILD_* (1)", "variables set: CARGO_PROFILE_* (1)",
                         "variable set: CARGO_BUILD_JOBS (not a plain job count)"):
            self.assertIn(expected, output)
        self.assertFalse(home.exists())
        self.assertEqual(github_env, "")


if __name__ == "__main__":
    unittest.main()
