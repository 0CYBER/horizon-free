"""tools/ci/cargo_isolation.py: no cargo configuration from outside the checkout, a fresh cargo home, no values in
its output."""
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

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
        environ = clean_environ(RUNNER_TEMP=str(runner_temp), GITHUB_ENV=str(github_env), **extra)
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
        self.assertTrue(result.stdout.endswith(cargo_isolation.CLEAN + "\n"), result.stdout)

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
            f"CARGO_REGISTRIES_{in_name}_TOKEN": "x", f"CARGO_ALIAS_{in_name}": "build"})
        output = result.stdout + result.stderr
        self.assertEqual(result.returncode, 1)
        self.assertNotIn(cargo_isolation.CLEAN, output)
        self.assertNotIn("s3cret", output.lower())
        self.assertNotIn("4711", output)
        for expected in ("(tables: build, patch, registries, other (1))", "could not be parsed",
                         "variables set: CARGO_REGISTRY_* (1)", "variables set: CARGO_REGISTRIES_* (1)",
                         "variables set: CARGO_ALIAS_* (1)", "variable set: RUSTC_WRAPPER"):
            self.assertIn(expected, output)
        self.assertFalse(home.exists())
        self.assertEqual(github_env, "")


if __name__ == "__main__":
    unittest.main()
