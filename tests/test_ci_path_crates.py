import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

from tests.test_ci_concurrency import ROOT, yaml

SCRIPT = ROOT / "tools/ci/path_crates.py"
# Three entries in Cargo's older target layout, one folder since 1.100.
MOVED = "handled [13] entries"


class PathCratesTests(unittest.TestCase):
    def setUp(self):
        if not shutil.which("cargo"):
            self.skipTest("Cargo is not installed")
        # The Cargo fixture's build output stays under this checkout's target.
        target = ROOT / "target"
        target.mkdir(exist_ok=True)
        self.directory = tempfile.TemporaryDirectory(prefix="path-crates-", dir=target)
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.write("Cargo.toml", '[workspace]\nresolver = "3"\nmembers = ["member", "patched-extra"]\n'
                                 'exclude = ["third_party/patched"]\n')
        self.package("third_party/patched", "patched", "pub fn value() {}\n")
        # A member whose name starts with the excluded crate's: rust-cache keeps it, so it stays in target.
        self.package("patched-extra", "patched-extra", "pub fn value() {}\n")
        self.package("member", "member", "pub fn value() { patched::value(); patched_extra::value() }\n",
                     'patched = { path = "../third_party/patched" }\npatched-extra = { path = "../patched-extra" }\n')
        # restore compares the crates with the commit main's build is of, as the workflow records it.
        self.write(".gitignore", "/target/\n/.ci-built/\n")
        subprocess.run(["git", "init", "-q"], cwd=self.root, check=True)
        self.commit("built")
        self.write(".ci-built/sha", subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=self.root, text=True))

    def commit(self, message):
        git = ["git", "-c", "user.name=test", "-c", "user.email=test@example.invalid", "-c", "core.autocrlf=false"]
        for command in (["add", "-A"], ["commit", "-q", "-m", message]):
            subprocess.run([*git, *command], cwd=self.root, check=True)

    def write(self, name, text):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)

    def package(self, directory, name, source, dependencies=""):
        self.write(f"{directory}/Cargo.toml",
                   f'[package]\nname = "{name}"\nversion = "0.0.0"\nedition = "2024"\n[dependencies]\n{dependencies}')
        self.write(f"{directory}/src/lib.rs", source)

    def check(self):
        env = {**os.environ, "CARGO_TARGET_DIR": str(self.root / "target"), "RUSTC_WRAPPER": ""}
        output = subprocess.check_output(["cargo", "check", "--offline", "--message-format=json"],
                                         cwd=self.root, env=env, stderr=subprocess.DEVNULL)
        return {item["target"]["name"]: item["fresh"] for line in output.splitlines()
                if (item := json.loads(line))["reason"] == "compiler-artifact"}

    def run_script(self, action):
        arguments = [action] if action == "prepare" else [action, ".ci-built"]
        return subprocess.check_output([sys.executable, str(SCRIPT), *arguments], cwd=self.root, text=True)

    def test_a_saved_path_crate_comes_back_fresh_and_keeps_what_is_above_it_fresh(self):
        self.assertIn("handled 0 entries", self.run_script("restore"))
        self.check()
        patched = self.root / "third_party/patched"
        self.assertRegex(self.run_script("save"), MOVED)
        # As after a restore of what rust-cache saved: the crate's build is back with its times.
        self.assertRegex(self.run_script("restore"), MOVED)
        self.assertEqual(self.check(), {"patched": True, "patched_extra": True, "member": True})
        # A pull request's files are all dated 2001: the manifest becomes the newest again, unless it changed.
        os.utime(patched / "Cargo.toml", (978307200, 978307200))
        changed = (patched / "src/lib.rs").stat().st_mtime
        self.run_script("save"), self.run_script("restore")
        self.assertEqual(int((patched / "Cargo.toml").stat().st_mtime), 978307201)
        self.assertEqual((patched / "src/lib.rs").stat().st_mtime, changed)
        self.assertEqual(self.check(), {"patched": True, "patched_extra": True, "member": True})
        # What rust-cache leaves on its own: without the crate's build, it and the member above it are checked again.
        self.assertRegex(self.run_script("save"), MOVED)
        self.assertEqual(self.check(), {"patched": False, "patched_extra": True, "member": False})
        # Main's next build starts without the crate's old build and dates its files as a pull request's are.
        self.assertRegex(self.run_script("prepare"), MOVED)
        self.assertEqual(self.check(), {"patched": False, "patched_extra": True, "member": False})
        self.assertEqual({path.name: int(path.stat().st_mtime) for path in [patched, *patched.rglob("*")]},
                         {"patched": 978307200, "src": 978307200, "lib.rs": 978307200, "Cargo.toml": 978307201})
        self.assertNotEqual(int((self.root / "member/src/lib.rs").stat().st_mtime), 978307200)

    def test_a_pull_request_that_changes_a_path_crate_rebuilds_it(self):
        self.check()
        self.assertRegex(self.run_script("save"), MOVED)
        # A pull request's commit changes a file Cargo might not look at: a C source behind a build script that names
        # its inputs.
        self.write("third_party/patched/c_src/alloc.c", "int broken(\n")
        self.commit("a pull request")
        self.assertIn("handled 0 entries", self.run_script("restore"))
        self.assertEqual(self.check(), {"patched": False, "patched_extra": True, "member": False})

    def test_both_build_jobs_prepare_or_restore_after_they_date_the_files_and_save_on_main(self):
        jobs = yaml.safe_load((ROOT / ".github/workflows/ci.yml").read_text())["jobs"]
        for job, build in (("verify-static", "cargo +stable clippy --workspace"), ("verify-test", "cargo +stable test")):
            with self.subTest(job=job):
                steps = jobs[job]["steps"]
                runs = [step.get("run", step.get("uses", "")) for step in steps]
                index = {name: next(i for i, run in enumerate(runs) if name in run)
                         for name in ("Swatinem/rust-cache", "pin_mtimes.sh .ci-built", "path_crates.py $ACTION",
                                      build, "path_crates.py save .ci-built")}
                self.assertEqual(sorted(index.values()), list(index.values()))
                step = steps[index["path_crates.py $ACTION"]]
                self.assertEqual(step["env"]["ACTION"],
                                 "${{ github.event_name == 'pull_request' && 'restore .ci-built' || 'prepare' }}")
                self.assertIn("github.event_name != 'merge_group'", step["if"])
                save = steps[index["path_crates.py save .ci-built"]]
                self.assertIn("git rev-parse HEAD > .ci-built/sha", save["run"])
                self.assertIn("github.ref == 'refs/heads/main'", save["if"])


if __name__ == "__main__":
    unittest.main()
