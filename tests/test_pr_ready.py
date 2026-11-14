import argparse
import contextlib
import io
import datetime
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import threading
import time
import unittest
from unittest import mock

from tools import pr_ready

PACKAGES = {"horizon-core": "crates/horizon-core", "horizon-client": "crates/horizon-client",
            "horizon-theme": "crates/horizon-theme", "xtask": "xtask"}
LIBRARIES = set(PACKAGES) - {"xtask"}
DEPENDENCIES = {"horizon-client": {"horizon-core"}}
MODS = ""
CLIPPY = f'status=0\n{pr_ready.CI_CLIPPY} || status=$?\n{pr_ready.CI_CLIPPY_SERVER} || status=$?\nexit "$status"'
# The bash on PATH: Windows' CreateProcess looks in System32 before PATH, so a bare "bash" there is WSL's
# (#5383), which has none of the host's paths. Git Bash on PATH runs the launcher's snippets as on Linux.
BASH = shutil.which("bash")
POSIX_BASH = BASH is not None and not (
    os.name == "nt" and os.path.dirname(os.path.normcase(BASH)).endswith(os.path.normcase(r"\system32")))


def args(host="local", quick=False, harnesses=False):
    return argparse.Namespace(host=host, quick=quick, harnesses=harnesses)


class GitChangedPathsTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.git("init")
        for key, value in (("core.quotePath", "true"), ("core.autocrlf", "false"),
                           ("core.hooksPath", str(self.root / "no-hooks")), ("commit.gpgsign", "false")):
            self.git("config", key, value)
        for module in (pr_ready, pr_ready.parity):
            patcher = mock.patch.object(module, "ROOT", self.root)
            patcher.start()
            self.addCleanup(patcher.stop)

    def git(self, *arguments):
        return subprocess.run(["git", *arguments], cwd=self.root, capture_output=True,
                              text=True, encoding="utf-8", check=True).stdout

    def write(self, path, text):
        destination = self.root / path
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_text(text, encoding="utf-8", newline="\n")

    def commit(self):
        self.git("add", ".")
        self.git("-c", "user.name=Test", "-c", "user.email=test@example.invalid", "commit", "-m", "base")

    @unittest.skipUnless(os.name == "posix", "requires POSIX symlinks and FIFO")
    def test_revision_snapshot_never_reads_symlink_targets_or_nonregular_files(self):
        self.write("tracked.txt", "base\n")
        self.commit()
        (self.root / "dangling").symlink_to("absent")
        (self.root / "directory-link").symlink_to(self.root, target_is_directory=True)
        os.mkfifo(self.root / "pipe")
        actual_git = pr_ready.git
        def git(*arguments):
            if arguments[0] == "ls-files":
                # Git may omit nonregular files; exercise the refusal even if listed.
                return "dangling\0directory-link\0pipe\0"
            return actual_git(*arguments)
        with mock.patch.object(pr_ready, "git", side_effect=git), \
                mock.patch.object(Path, "open", side_effect=AssertionError("must not read untracked objects")):
            before = pr_ready.revision_state()
            self.assertTrue(before[1])
            self.assertIn("absent", before[2])
            self.assertIn("pipe", before[2])
            (self.root / "dangling").unlink()
            (self.root / "dangling").symlink_to("another-absent")
            after = pr_ready.revision_state()
        self.assertNotEqual(before, after)

    def test_self_check_header_identifies_commit_and_dirty_state(self):
        self.write("tracked.txt", "base\n")
        self.write(".gitignore", "ignored-output/\n")
        self.commit()
        head = self.git("rev-parse", "HEAD").strip()
        def header():
            return pr_ready.block(args(), "HEAD", head, [], {}, [], "skip-release", 0,
                                  pr_ready.revision_state()).splitlines()[0]
        self.assertIn(f"commit `{head}`", header())
        self.assertIn("host `", header())
        self.assertNotIn(" dirty", header())
        self.write("ignored-output/log.txt", "ignored\n")
        self.assertNotIn(" dirty", header())
        self.write("untracked.txt", "new\n")
        self.assertIn(f"commit `{head}` dirty", header())
        (self.root / "untracked.txt").unlink()
        self.write("tracked.txt", "edited\n")
        self.assertIn(" dirty", header())
        self.git("add", "tracked.txt")
        self.assertIn(" dirty", header())

    def test_unicode_paths_select_checks_and_keep_line_counts(self):
        tracked = "crates/horizon-core/src/caf\u00e9 edit.rs"
        untracked = "crates/horizon-core/src/\u65b0 file.rs"
        self.write(tracked, "first\n")
        self.commit()
        self.write(tracked, "first\nsecond\n")
        self.write(untracked, "new\nfile\n")
        changed = pr_ready.parity.changed_files("HEAD")
        self.assertEqual(changed, {tracked, untracked})
        self.assertEqual(pr_ready.line_counts("HEAD", changed), {tracked: (1, 0), untracked: (2, 0)})
        self.assertTrue(all(pr_ready.workspace_rust(path) for path in changed))
        self.assertEqual(pr_ready.touched_crates(changed, PACKAGES), ["horizon-core"])

    def test_renamed_rust_source_keeps_both_paths_and_counts(self):
        before, after = "crates/horizon-core/src/moved.rs", "docs/moved.md"
        self.write(before, "fn moved() {\n}\n")
        self.commit()
        (self.root / "docs").mkdir()
        self.git("mv", before, after)
        changed = pr_ready.parity.changed_files("HEAD")
        self.assertEqual(changed, {before, after})
        self.assertEqual(pr_ready.line_counts("HEAD", changed), {before: (0, 2), after: (2, 0)})
        self.assertEqual(pr_ready.touched_crates(changed, PACKAGES), ["horizon-core"])

    @unittest.skipIf(os.name == "nt", "Windows filenames cannot contain tabs or newlines")
    def test_tabs_and_newlines_remain_in_paths(self):
        tracked = "crates/horizon-core/src/tab\tfile.rs"
        untracked = "crates/horizon-core/src/line\r\nfile.rs"
        self.write(tracked, "first\n")
        self.commit()
        self.write(tracked, "first\nsecond\n")
        self.write(untracked, "new\n")
        changed = pr_ready.parity.changed_files("HEAD")
        self.assertEqual(changed, {tracked, untracked})
        self.assertEqual(pr_ready.line_counts("HEAD", changed), {tracked: (1, 0), untracked: (1, 0)})


class PrReadyTests(unittest.TestCase):
    @unittest.skipUnless(POSIX_BASH and os.name != "nt", "executes the POSIX self-check")
    def test_cold_clippy_hint_precedes_the_step_and_respects_sccache(self):
        for wrapper in (None, "", "other-wrapper", "sccache", "/opt/bin/sccache", "sccache.exe"):
            with self.subTest(wrapper=wrapper):
                env = dict(os.environ)
                env.pop("RUSTC_WRAPPER", None)
                if wrapper is not None:
                    env["RUSTC_WRAPPER"] = wrapper
                steps = [pr_ready.Step(name, "printf 'ran\\n'")
                         for name in ("clippy", "registry clippy", "launcher clippy")]
                with mock.patch.dict(os.environ, env, clear=True), contextlib.redirect_stderr(io.StringIO()) as log:
                    pr_ready.run_script(steps, "local", verbose=False)
                self.assertEqual([(step.status, step.output.strip()) for step in steps], [("pass", "ran")] * 3)
                expected = wrapper in (None, "", "other-wrapper")
                self.assertEqual(log.getvalue().count(pr_ready.CLIPPY_CACHE_HINT), int(expected))
                if expected:
                    self.assertLess(log.getvalue().index(pr_ready.CLIPPY_CACHE_HINT), log.getvalue().index("  clippy..."))
        step = pr_ready.Step("tests", "printf 'ran\\n'")
        with mock.patch.dict(os.environ, {"RUSTC_WRAPPER": ""}), contextlib.redirect_stderr(io.StringIO()) as log:
            pr_ready.run_script([step], "local", verbose=False)
        self.assertNotIn(pr_ready.CLIPPY_CACHE_HINT, log.getvalue())

    def test_data_backed_summary_with_and_without_prepared_data(self):
        with tempfile.TemporaryDirectory() as directory, \
                mock.patch.object(pr_ready, "ROOT", Path(directory) / "checkout/repo"), \
                mock.patch.object(pr_ready, "git", side_effect=lambda *args: "a" * 40 if args[0] == "rev-parse" else ""), \
                mock.patch.dict(os.environ):
            os.environ.pop("HORIZON_TEST_RESOURCES", None)
            step = pr_ready.Step("content root tests", summary=pr_ready.tests_passed)
            step.output = "test result: ok. 3 passed; 0 failed; 0 ignored"
            step.status = "pass"
            step.detail = pr_ready.tests_passed(step.output)
            def render():
                return pr_ready.block(args(), "origin/main", "abcdef123", [], {}, [step], "skip-release", 0,
                                      ("a" * 40, "", ""))
            self.assertIn("data-backed tests not run: no prepared data here", render())
            prepared = pr_ready.ROOT / ".local/horizon-client/26.3"
            prepared.mkdir(parents=True)
            self.assertIn("data-backed tests not run: no prepared data here", render())
            (prepared / "gameplay-facts.json").write_text("{}")
            self.assertIn("data-backed tests: horizon-client-lib mod_content::tests:: (--ignored): 3 passed", render())
            override = Path(directory) / "override"
            override.mkdir()
            os.environ["HORIZON_TEST_RESOURCES"] = str(override)
            self.assertIn("data-backed tests not run: no prepared data here", render())
            (override / "gameplay-facts.json").write_text("{}")
            (prepared / "gameplay-facts.json").unlink()
            self.assertIn("data-backed tests: horizon-client-lib mod_content::tests:: (--ignored): 3 passed", render())
            step.status, step.detail = "skip", "content root untouched"
            self.assertIn("data-backed tests not run: content root untouched", render())

    def test_windows_runner_gives_a_leading_variable_to_the_environment(self):
        step = pr_ready.Step(
            "dispatch tests tests",
            "CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-4} cargo test --manifest-path a/Cargo.toml --locked",
        )
        with mock.patch.object(pr_ready.subprocess, "Popen") as popen, \
                mock.patch.object(pr_ready, "finish"), mock.patch.dict(os.environ):
            os.environ.pop("CARGO_BUILD_JOBS", None)
            popen.return_value.__enter__.return_value.stdout = []
            popen.return_value.__enter__.return_value.wait.return_value = 0
            popen.return_value.__enter__.return_value.returncode = 0
            pr_ready.run_windows([step], False)
        self.assertEqual(popen.call_args.args[0],
                         ["cargo", "test", "--manifest-path", "a/Cargo.toml", "--locked"])
        self.assertEqual(popen.call_args.kwargs["env"]["CARGO_BUILD_JOBS"], "4")
        self.assertEqual(step.status, "pass")

    def test_paths_map_to_the_crates_that_own_them(self):
        self.assertEqual(pr_ready.touched_crates({"crates/horizon-core/src/jmath.rs", "crates/horizon-client/build.rs",
                                                  "xtask/src/main.rs", "tools/parity.py"}, PACKAGES),
                         ["horizon-client", "horizon-core", "xtask"])
        self.assertEqual(pr_ready.touched_crates({"crates/horizon-core/README.md"}, PACKAGES), [])
        # The generated stylesheet is checked by horizon-theme's tests.
        self.assertEqual(pr_ready.touched_crates({"crates/horizon-theme/tokens.css"}, PACKAGES), ["horizon-theme"])
        # A crate whose name prefixes another's directory doesn't own it.
        self.assertEqual(pr_ready.touched_crates({"crates/horizon-core-extra/x.rs"}, PACKAGES), [])
        # The workspace manifest and lock touch every crate.
        self.assertIsNone(pr_ready.touched_crates({"Cargo.lock", "crates/horizon-core/src/lib.rs"}, PACKAGES))

    def test_shared_client_sources_select_the_library_without_gameplay(self):
        packages = {**PACKAGES, "horizon-client-lib": "crates/horizon-client-lib"}
        for path in ["src/client_lib.rs", "src/health.rs", "src/lan/hub.rs", "src/bin/horizon-ctl/main.rs",
                     "examples/probe/main.rs", "tests/lan_bot.rs"]:
            self.assertEqual(pr_ready.touched_crates({f"crates/horizon-client/{path}"}, packages),
                             ["horizon-client-lib"])
        for path in ["src/lib.rs", "src/main.rs", "src/entry.rs", "src/game.rs", "src/game/items/combat.rs",
                     "tests/launch_api.rs", "Cargo.toml", "build.rs"]:
            self.assertEqual(pr_ready.touched_crates({f"crates/horizon-client/{path}"}, packages),
                             ["horizon-client"])

    def test_which_workspace_builds_a_path(self):
        self.assertTrue(pr_ready.workspace_rust("crates/horizon-theme/fonts/x.ttf"))
        self.assertTrue(pr_ready.workspace_rust("Cargo.toml"))
        self.assertFalse(pr_ready.workspace_rust("crates/horizon-client/README.md"))
        self.assertFalse(pr_ready.workspace_rust("launcher/src/main.rs"))
        self.assertTrue(pr_ready.launcher_rust("launcher/src/main.rs"))
        # CI's launcher job also runs for the root workspace's manifests and for its own check.
        self.assertTrue(pr_ready.launcher_rust("Cargo.lock"))
        self.assertTrue(pr_ready.launcher_rust("tools/ci/rust_changes.py"))
        self.assertTrue(pr_ready.launcher_rust("crates/horizon-prepare/src/lib.rs"))
        self.assertTrue(pr_ready.launcher_rust("crates/horizon-theme/src/lib.rs"))
        self.assertTrue(pr_ready.launcher_rust("crates/horizon-system/src/lib.rs"))
        self.assertTrue(pr_ready.launcher_rust("crates/horizon-mod-api-gen/src/injection.rs"))
        self.assertFalse(pr_ready.launcher_rust("launcher/links.json"))
        self.assertFalse(pr_ready.launcher_rust("launcher/README.md"))
        self.assertFalse(pr_ready.launcher_rust("crates/horizon-client/src/game.rs"))

    def test_crate_checks_do_not_link_unrelated_examples(self):
        with mock.patch.object(pr_ready, "affected_here", return_value=[]):
            steps = pr_ready.plan(args(quick=True), "origin/main", {"crates/horizon-client/src/game.rs"}, PACKAGES, LIBRARIES, DEPENDENCIES)
        by_name = {step.name: step for step in steps}
        self.assertEqual(by_name["clippy"].command, CLIPPY)
        self.assertEqual(by_name["tests: horizon-client"].command,
                         "cargo test --locked -p horizon-client --tests")
        self.assertEqual(by_name["doc tests"].command, "cargo test --locked -p horizon-client --doc")
        self.assertIsNone(by_name["doc tests"].status)

    def test_file_size_is_skipped_only_on_a_shallow_clone(self):
        def file_size_step(shallow):
            with mock.patch.object(pr_ready, "affected_here", return_value=[]), \
                    mock.patch.object(pr_ready.subprocess, "run",
                                      return_value=subprocess.CompletedProcess([], 0, stdout=shallow)) as run:
                steps = pr_ready.plan(args(quick=True), "origin/main", {"tools/pr_ready.py"}, PACKAGES, LIBRARIES, DEPENDENCIES)
            run.assert_called_once_with(["git", "rev-parse", "--is-shallow-repository"], cwd=pr_ready.ROOT,
                                        capture_output=True, text=True)
            return {step.name: step for step in steps}["File size"]
        shallow = file_size_step("true\n")
        self.assertEqual((shallow.status, shallow.detail), ("skip", "not checked on a shallow clone"))
        full = file_size_step("false\n")
        self.assertIsNone(full.status)
        self.assertEqual(full.command, "python3 tools/check_file_size.py --base origin/main")

    def test_content_root_tests_run_only_when_what_they_load_changes(self):
        def content_step(path):
            with mock.patch.object(pr_ready, "affected_here", return_value=[]):
                steps = pr_ready.plan(args(quick=True), "origin/main", {path}, PACKAGES, LIBRARIES, DEPENDENCIES)
            return {step.name: step for step in steps}["content root tests"]
        self.assertEqual(content_step("crates/horizon-client/src/game.rs").skip, "content root untouched")
        for path in ["crates/horizon-client/src/mod_content.rs", "crates/horizon-client/src/mod_content_tests.rs",
                     "crates/horizon-mod-package/src/content.rs", "crates/horizon-assets/src/lib.rs", "Cargo.lock"]:
            with self.subTest(path=path):
                step = content_step(path)
                self.assertIsNone(step.skip)
                self.assertEqual(step.command, pr_ready.CONTENT_ROOT_TESTS)

    def test_dispatch_and_registry_suites_run_when_their_inputs_change(self):
        def suite_steps(path):
            with mock.patch.object(pr_ready, "affected_here", return_value=[]):
                steps = pr_ready.plan(args(quick=True), "origin/main", {path}, PACKAGES, LIBRARIES, DEPENDENCIES)
            return {step.name: step for step in steps}
        for path in ["crates/horizon-mod-api-gen/src/lib.rs",
                     "crates/horizon-mod-api-gen/dispatch-tests/src/lib.rs",
                     "crates/horizon-mod-api-macros/src/lib.rs",
                     "crates/horizon-mod-vocab/src/lib.rs",
                     "crates/horizon-mod-vocab/src/parts/mod.rs",
                     "crates/horizon-mod-vocab/src/hooks/block.rs"]:
            with self.subTest(path=path):
                by_name = suite_steps(path)
                for name in ("dispatch tests clippy", "dispatch tests tests"):
                    self.assertIsNone(by_name[name].skip)
                self.assertIn("crates/horizon-mod-api-gen/dispatch-tests/Cargo.toml",
                              by_name["dispatch tests clippy"].command)
                self.assertIn("--test-threads=4", by_name["dispatch tests tests"].command)
                self.assertEqual(by_name["mod registry tests clippy"].skip, "mod registry untouched")
        for path in ["crates/horizon-mod-registry/src/main.rs", "crates/horizon-mod-registry/Cargo.toml"]:
            with self.subTest(path=path):
                by_name = suite_steps(path)
                for name in ("mod registry tests clippy", "mod registry tests tests"):
                    self.assertIsNone(by_name[name].skip)
                self.assertEqual(by_name["dispatch tests clippy"].skip, "dispatch surface untouched")
        by_name = suite_steps("crates/horizon-core/src/lib.rs")
        self.assertEqual(by_name["dispatch tests clippy"].skip, "dispatch surface untouched")
        self.assertEqual(by_name["mod registry tests tests"].skip, "mod registry untouched")

    def test_shared_rust_configuration_keeps_workspace_coverage(self):
        for path in sorted(pr_ready.ROOT_RUST | {".cargo/config.toml", "crates/unknown/src/lib.rs"}):
            with self.subTest(path=path), mock.patch.object(pr_ready, "affected_here", return_value=[]):
                steps = pr_ready.plan(args(quick=True), "origin/main", {path}, PACKAGES, LIBRARIES, DEPENDENCIES)
                by_name = {step.name: step for step in steps}
                self.assertEqual(by_name["clippy"].command, CLIPPY)
                self.assertEqual(by_name["tests: workspace"].command,
                                 "cargo test --locked --workspace" + MODS + " --tests")
                self.assertEqual(by_name["doc tests"].command, "cargo test --locked --workspace --doc")

    def test_holder_actor_is_built_before_applicable_tests(self):
        packages = {**PACKAGES, "horizon-mod-package": "crates/horizon-mod-package"}
        for path in ("Cargo.lock", "crates/unknown/src/lib.rs",
                     "crates/horizon-mod-package/src/holder.rs", "launcher/src/main.rs"):
            with self.subTest(path=path), mock.patch.object(pr_ready, "affected_here", return_value=[]):
                steps = pr_ready.plan(args(quick=True), "origin/main", {path}, packages,
                                      LIBRARIES, DEPENDENCIES)
                by_name = {step.name: step for step in steps}
                actor = by_name["holder actor"]
                self.assertIsNone(actor.skip)
                self.assertEqual(actor.command, "cargo build --locked -p horizon-mod-package "
                                 "--features runtime-process --example holder_actor")
                # No cd, target-dir or profile override: actor() finds the sibling
                # examples directory in the root test target, including CARGO_TARGET_DIR.
                self.assertLess(steps.index(actor), next(i for i, step in enumerate(steps)
                                if step.name.startswith("tests")))
                self.assertLess(steps.index(actor), steps.index(by_name["launcher tests"]))
        for path in ("README.md", "crates/horizon-core/src/lib.rs"):
            with self.subTest(path=path), mock.patch.object(pr_ready, "affected_here", return_value=[]):
                steps = pr_ready.plan(args(quick=True), "origin/main", {path}, packages,
                                      LIBRARIES, DEPENDENCIES)
                self.assertEqual(next(step for step in steps if step.name == "holder actor").skip,
                                 "holder tests untouched")

    def test_tests_get_the_features_ci_s_workspace_run_gives_them(self):
        self.assertEqual(pr_ready.mod_features(["horizon-world"]), "")
        self.assertEqual(pr_ready.mod_features(["horizon-client"]), "")
        self.assertEqual(pr_ready.mod_features(["horizon-mod-package"]),
                         " --features horizon-mod-package/index")
        self.assertEqual(pr_ready.mod_features(None), MODS)

    def test_mod_api_data_inputs_build_and_test_the_consuming_crates(self):
        packages = {**PACKAGES, "horizon-client-render": "crates/horizon-client-render",
                    "horizon-mod-api-gen": "crates/horizon-mod-api-gen"}
        for path in ("crates/horizon-mod-vocab/src/hooks/player.rs",):
            with self.subTest(path=path), mock.patch.object(pr_ready, "affected_here", return_value=[]):
                steps = pr_ready.plan(args(quick=True), "origin/main", {path}, packages,
                                      LIBRARIES | {"horizon-client-render", "horizon-mod-api-gen"}, DEPENDENCIES)
                by_name = {step.name: step for step in steps}
                self.assertIsNone(by_name["clippy"].skip)
                self.assertIsNone(by_name["rustfmt"].skip)
                self.assertEqual(by_name["rustfmt"].command, "python3 tools/ci/format_scan.py")
                tests = next(step for step in steps if step.name.startswith("tests:"))
                self.assertIsNone(tests.skip)
                self.assertEqual(tests.command, "cargo test --locked -p horizon-client -p horizon-client-render "
                                               "-p horizon-mod-api-gen --tests")

    def test_shared_generator_edits_run_launcher_checks(self):
        with mock.patch.object(pr_ready, "affected_here", return_value=[]):
            steps = pr_ready.plan(args(quick=True), "origin/main",
                                  {"crates/horizon-mod-api-gen/src/injection.rs"},
                                  {**PACKAGES, "horizon-mod-api-gen": "crates/horizon-mod-api-gen"},
                                  LIBRARIES | {"horizon-mod-api-gen"}, DEPENDENCIES)
        by_name = {step.name: step for step in steps}
        for name in ("launcher rustfmt", "launcher clippy", "launcher tests"):
            self.assertIsNone(by_name[name].skip)

    @unittest.skipUnless(os.name == "nt", "the Windows runner's own jobs")
    def test_a_windows_check_ends_what_it_left_running(self):
        """A check that exits leaving a child that holds its output: the run goes on, and the child is ended."""
        leaving = ("import subprocess, sys; subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(120)']);"
                   " print('left one')")
        step = pr_ready.Step("leaving", f"python3 -c \"{leaving}\"")
        started = time.monotonic()
        pr_ready.run_windows([step], verbose=False)
        self.assertLess(time.monotonic() - started, 60)
        self.assertEqual((step.status, step.output.strip()), ("pass", "left one"))

    @unittest.skipUnless(shutil.which("cargo"), "needs cargo")
    def test_windows_launcher_checks_include_binary_tests(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            launcher = root / "launcher"
            (launcher / "src").mkdir(parents=True)
            (launcher / "Cargo.toml").write_text(
                '[package]\nname = "launcher-check-fixture"\nversion = "0.1.0"\nedition = "2021"\n[workspace]\n')
            (launcher / "src/lib.rs").write_text('#[test]\nfn library_passes() {}\n')
            (launcher / "src/main.rs").write_text(
                'fn main() {}\n#[test]\nfn binary_fails() { panic!("launcher binary test ran"); }\n')
            with mock.patch.object(pr_ready, "ROOT", root), \
                    mock.patch.dict(os.environ, {"CARGO_TARGET_DIR": str(root / "target")}):
                subprocess.run(["cargo", "generate-lockfile", "--offline"], cwd=launcher, check=True,
                               capture_output=True)
                step = pr_ready.Step("launcher tests", "cargo test --locked --lib --bins")
                pr_ready.run_windows([step], verbose=False)
            self.assertEqual(step.status, "fail")
            self.assertIn("launcher binary test ran", step.output)

    @unittest.skipUnless(POSIX_BASH, "needs a bash that is not WSL's")
    def test_the_launcher_fallback_asks_for_a_feature_the_launcher_has(self):
        """The macOS fallback must name gpui_platform/runtime_shaders: gpui_platform is launcher/Cargo.toml's
        rename of gpui-fast-platform, whose runtime_shaders reaches gpui_macos; the gpui rename (gpui-fast)
        has no such feature, and neither has Linux any fallback."""
        with mock.patch.object(pr_ready, "affected_here", return_value=[]):
            steps = pr_ready.plan(args(quick=True), "origin/main", {"launcher/src/main.rs"},
                                  PACKAGES, LIBRARIES, DEPENDENCIES)
        command = {step.name: step for step in steps}["launcher clippy"].command
        for uname, metal, features in (("Darwin", False, ["--features", "gpui_platform/runtime_shaders"]),
                                        ("Darwin", True, []), ("Linux", False, [])):
            with self.subTest(uname=uname, metal=metal):
                self.assertEqual(self.launcher_step_arguments(command, uname, metal),
                                 ["clippy", "--keep-going", "--all-targets", "--locked", *features,
                                  "--message-format=json-diagnostic-rendered-ansi", "--", "--cap-lints=warn"])

    def launcher_step_arguments(self, command, uname, metal):
        """Run a launcher step's shell snippet with fake uname, xcrun, pkg-config and cargo; what the fake
        cargo receives is the snippet's observable behaviour."""
        with tempfile.TemporaryDirectory() as fake_bin:
            scripts = {"uname": f"#!/bin/sh\necho {uname}\n",
                       "xcrun": f"#!/bin/sh\nexit {0 if metal else 1}\n",
                       # xkbcommon found: Linux never looks for nix-shell (a NixOS host would have it).
                       "pkg-config": "#!/bin/sh\nexit 0\n",
                       "cargo": '#!/bin/sh\nif [ "$1" = metadata ]; then echo \'{"workspace_members": []}\'; else printf \'%s\\n\' "$@"; fi\n'}
            for name, script in scripts.items():
                path = os.path.join(fake_bin, name)
                # LF on every host: text mode makes the shebang "#!/bin/sh\r" on
                # Windows, which bash cannot run, so the fake names are skipped.
                with open(path, "w", newline="\n") as file:
                    file.write(script)
                os.chmod(path, 0o755)
            env = dict(os.environ)
            if os.name == "nt":
                # Git Bash puts /usr/bin ahead of every Windows PATH entry, so a fake
                # named uname can only win when the shell itself puts its folder first.
                command = 'export PATH="$(cygpath -u "$FAKE_BIN"):$PATH"\n' + command
                env["FAKE_BIN"] = fake_bin
            else:
                env["PATH"] = fake_bin + os.pathsep + os.environ["PATH"]
            return subprocess.run([BASH, "-c", command], capture_output=True, text=True,
                                  cwd=pr_ready.ROOT, env=env, check=True).stdout.splitlines()

    def test_python_tests_always_run(self):
        with mock.patch.object(pr_ready, "affected_here", return_value=[]):
            steps = pr_ready.plan(args(quick=True), "origin/main", {"crates/horizon-core/src/lib.rs"},
                                  PACKAGES, LIBRARIES, DEPENDENCIES)
        self.assertIsNone({step.name: step for step in steps}["Python tests"].status)

    def test_non_rust_changes_skip_workspace_checks(self):
        for path in ("README.md", "tools/pr_ready.py", "launcher/src/main.rs"):
            with self.subTest(path=path), mock.patch.object(pr_ready, "affected_here", return_value=[]):
                steps = pr_ready.plan(args(quick=True), "origin/main", {path}, PACKAGES, LIBRARIES, DEPENDENCIES)
                by_name = {step.name: step for step in steps}
                for name in ("clippy", "tests", "doc tests"):
                    self.assertEqual(by_name[name].status, "skip")

    def test_multiple_touched_crates_share_the_same_check_scope(self):
        with mock.patch.object(pr_ready, "affected_here", return_value=[]):
            steps = pr_ready.plan(args(), "origin/main",
                                  {"crates/horizon-core/src/lib.rs", "crates/horizon-client/examples/probe.rs"}, PACKAGES, LIBRARIES, DEPENDENCIES)
        for step in steps:
            if step.name in ("tests: horizon-client, horizon-core", "doc tests"):
                self.assertIn("-p horizon-client -p horizon-core", step.command)
                self.assertIsNone(step.status)

    def test_binary_only_crates_skip_doc_tests(self):
        with mock.patch.object(pr_ready, "affected_here", return_value=[]):
            steps = pr_ready.plan(args(), "origin/main", {"xtask/src/main.rs"}, PACKAGES, LIBRARIES, DEPENDENCIES)
        by_name = {step.name: step for step in steps}
        self.assertEqual(by_name["tests: xtask"].command, "cargo test --locked -p xtask --tests")
        self.assertIsNone(by_name["tests: xtask"].status)
        self.assertEqual(by_name["doc tests"].status, "skip")
        with mock.patch.object(pr_ready, "affected_here", return_value=[]):
            steps = pr_ready.plan(args(), "origin/main",
                                  {"xtask/src/main.rs", "crates/horizon-core/src/lib.rs"}, PACKAGES, LIBRARIES, DEPENDENCIES)
        by_name = {step.name: step for step in steps}
        self.assertEqual(by_name["doc tests"].command, "cargo test --locked -p horizon-core --doc")

    def test_clippy_is_ci_s_and_the_server_build_when_the_game_is_affected(self):
        packages = {name: f"crates/{name}" for name in ("core", "world", "horizon-client", "unrelated")}
        dependencies = {"world": {"core"}, "horizon-client": {"world"}, "unrelated": set()}
        with mock.patch.object(pr_ready, "affected_here", return_value=[]):
            steps = pr_ready.plan(args(), "origin/main", {"crates/core/src/lib.rs"},
                                  packages, set(packages), dependencies)
            unrelated = pr_ready.plan(args(), "origin/main", {"crates/unrelated/src/lib.rs"},
                                      packages, set(packages), dependencies)
        self.assertEqual({step.name: step for step in unrelated}["clippy"].command, pr_ready.CI_CLIPPY)
        by_name = {step.name: step for step in steps}
        self.assertEqual(by_name["clippy"].command, CLIPPY)
        self.assertEqual(by_name["tests: core"].command, "cargo test --locked -p core --tests")
        self.assertEqual(by_name["doc tests"].command, "cargo test --locked -p core --doc")

    def test_dependent_scope_handles_cycles_and_multiple_changed_crates(self):
        dependencies = {"a": {"b"}, "b": {"a"}, "c": {"b"}, "d": set()}
        self.assertEqual(pr_ready.dependent_crates(["a"], dependencies), ["a", "b", "c"])
        self.assertEqual(pr_ready.dependent_crates(["a", "d"], dependencies), ["a", "b", "c", "d"])
        self.assertEqual(pr_ready.dependent_crates([], dependencies), [])
        self.assertIsNone(pr_ready.dependent_crates(None, dependencies))

    def test_metadata_includes_renamed_optional_platform_build_and_dev_consumers(self):
        core = str(pr_ready.ROOT / "crates/core")
        metadata = {"workspace_members": ["core", "optional", "build", "dev"], "packages": [
            {"id": "core", "name": "core", "manifest_path": core + "/Cargo.toml",
             "targets": [{"kind": ["lib"]}], "dependencies": []},
        ]}
        for name, kind in (("optional", None), ("build", "build"), ("dev", "dev")):
            metadata["packages"].append({
                "id": name, "name": name, "manifest_path": str(pr_ready.ROOT / f"crates/{name}/Cargo.toml"),
                "targets": [{"kind": ["bin"]}], "dependencies": [
                    {"name": "core", "rename": "alias", "path": core, "kind": kind,
                     "optional": name == "optional", "target": "cfg(windows)"},
                    {"name": "external", "path": None},
                ],
            })
        # A path dependency outside this workspace must not become a selectable package.
        metadata["packages"].append({"id": "outside", "name": "outside", "manifest_path": "/outside/Cargo.toml"})
        with mock.patch.object(pr_ready.subprocess, "run", return_value=mock.Mock(stdout=json.dumps(metadata))):
            directories, libraries, dependencies = pr_ready.packages()
        self.assertEqual(set(directories), {"core", "optional", "build", "dev"})
        self.assertEqual(libraries, {"core"})
        self.assertEqual(pr_ready.dependent_crates(["core"], dependencies), ["build", "core", "dev", "optional"])

    def test_scope_warns_on_mixed_areas(self):
        self.assertEqual(pr_ready.scope_warnings({"crates/horizon-theme/src/lib.rs", "launcher/src/theme.rs"}), [])
        self.assertEqual(pr_ready.scope_warnings({"crates/horizon-client/src/game.rs", "launcher/src/main.rs",
                                                  ".github/workflows/ci.yml"}),
                         ["mixes CI, game, launcher"])

    def test_cargo_lock_changes_only_with_a_manifest(self):
        self.assertIsNone(pr_ready.lock_problem({"crates/horizon-core/src/lib.rs"}))
        self.assertIsNone(pr_ready.lock_problem({"Cargo.lock", "crates/horizon-client/Cargo.toml"}))
        self.assertIsNone(pr_ready.lock_problem({"launcher/Cargo.lock", "crates/horizon-theme/Cargo.toml"}))
        self.assertIn("Cargo.lock", pr_ready.lock_problem({"Cargo.lock", "crates/horizon-core/src/lib.rs"}))

    def test_label_follows_what_players_get(self):
        self.assertEqual(pr_ready.suggested_label({"tools/pr_ready.py", "CONTRIBUTING.md"}), "`skip-release`")
        self.assertEqual(pr_ready.suggested_label({"crates/horizon-world/examples/probe.rs"}), "`skip-release`")
        self.assertIn("feature", pr_ready.suggested_label({"crates/horizon-client/src/game.rs"}))
        self.assertIn("feature", pr_ready.suggested_label({"launcher/src/main.rs"}))

    def test_affected_harnesses_come_from_the_parity_runner(self):
        found = {"food": "tools/test_food_parity.py", "datafix#1": "tools/test_datafix_parity.py",
                 "datafix#2": "tools/test_datafix_parity.py"}
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            # Exercise the real selector/SourceGraph on the records' tiny workspace, not
            # every crate in this checkout. All three selection cases share this input.
            (root / "Cargo.toml").write_text('[workspace]\nmembers = ["crates/horizon-world"]\n')
            crate = root / "crates/horizon-world"
            (crate / "src").mkdir(parents=True)
            (crate / "Cargo.toml").write_text(
                '[package]\nname = "horizon-world"\nversion = "0.0.0"\n')
            (crate / "src/lib.rs").write_text('pub mod food;\npub mod datafix;\n')
            for module in ("food", "datafix"):
                (crate / f"src/{module}.rs").write_text('pub fn run() {}\n')
            found = {name: str(root / script) for name, script in found.items()}
            with mock.patch.object(pr_ready.parity, "harnesses", return_value=found), \
                    mock.patch.object(pr_ready.parity, "load_state", return_value={"harnesses": {
                        "food": {"dependencies": ["crates/horizon-world/src/food.rs"]},
                        "datafix#1": {"dependencies": ["crates/horizon-world/src/datafix.rs"]},
                        "datafix#2": {"dependencies": ["crates/horizon-world/src/datafix.rs"]}}}), \
                    mock.patch.object(pr_ready.parity, "ROOT", root), \
                    mock.patch.object(pr_ready.parity, "SHARED", root / "cache"), \
                    mock.patch.object(pr_ready.parity, "STATE_DIR", root / "cache"):
                self.assertEqual(pr_ready.affected_here({"crates/horizon-world/src/datafix.rs"}), ["datafix"])
                self.assertEqual(pr_ready.affected_here({"README.md"}), [])
                self.assertEqual(pr_ready.affected_here({"Cargo.lock"}), ["food", "datafix"])

    def test_actual_sources_place_food_probe_and_select_reached_source(self):
        # Parse the checkout's actual manifest, probe and reached implementation,
        # not another synthetic Rust program. A narrow root avoids rebuilding the
        # whole-workspace graph whose cost this test-only change removes.
        parity = pr_ready.parity
        probe = "crates/horizon-world/examples/probe/food_fixture.rs"
        source = "crates/horizon-world/src/food.rs"
        dependencies = {
            "crates/horizon-world/examples/probe/main.rs", probe,
            "crates/horizon-world/src/lib.rs", source,
        }
        artifacts = [("example", "probe", None, ["-p", "horizon-world"])]
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for path in dependencies | {"crates/horizon-world/Cargo.toml"}:
                destination = root / path
                destination.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(pr_ready.ROOT / path, destination)
            graph = parity.parity_reach.SourceGraph(root, parity.rust_code)
            entries = graph.entries(artifacts, dependencies)
            self.assertIn(probe, entries or (), "the actual probe must be placed, not fallback-selected")
            self.assertIn(source, graph.reachable(entries, dependencies),
                          "the actual food probe must reach its food implementation")

    def test_harnesses_are_listed_when_they_cannot_run(self):
        changed = {"crates/horizon-world/src/food.rs"}
        with mock.patch.object(pr_ready, "affected_here", return_value=["food", "light"]):
            steps = pr_ready.plan(args(quick=True), "origin/main", changed, {"horizon-world": "crates/horizon-world"}, {"horizon-world"}, {})
        harness = steps[-1]
        self.assertEqual(harness.status, "list")
        self.assertIn("2 affected, to run in CI (--quick): food, light", harness.detail)
        by_name = {step.name: step for step in steps}
        self.assertIn("tests: horizon-world", by_name)
        self.assertEqual(by_name["launcher clippy"].status, "skip")
        self.assertIsNone(by_name["clippy"].status)
        with mock.patch.object(pr_ready, "affected_here", return_value=["food"]):
            steps = pr_ready.plan(args(host="macmini", harnesses=True), "origin/main", changed, {"horizon-world": "crates/horizon-world"}, {"horizon-world"}, {})
        self.assertIn("--changed-files", steps[-1].command)
        self.assertIn("crates/horizon-world/src/food.rs", steps[-1].command)

    def test_harnesses_run_only_where_ci_s_parity_job_would(self):
        packages = {"horizon-world": "crates/horizon-world"}
        code = {"crates/horizon-world/src/food.rs"}
        self.assertFalse(pr_ready.runs_parity(code))
        for path in ("tools/test_food_parity.py", "tools/parity.py", "tools/lab.py", "tools/parity_hosts.json",
                     "mods/horizon/src/test/java/dev/horizonlab/parity/VanillaFoodOracle.java",
                     "crates/horizon-world/examples/food_probe.rs", "crates/horizon-client/src/game/sculk_fixture.rs",
                     ".github/workflows/parity-main.yml"):
            with self.subTest(path=path):
                self.assertTrue(pr_ready.runs_parity(code | {path}))
        with mock.patch.object(pr_ready, "affected_here", return_value=["food", "light"]):
            steps = pr_ready.plan(args(host="macmini"), "origin/main", code, packages, {"horizon-world"}, {})
            self.assertEqual(steps[-1].status, "list")
            self.assertIsNone(steps[-1].command)
            self.assertIn("2 affected, left to parity-main on main", steps[-1].detail)
            self.assertIn("--harnesses runs them here", steps[-1].detail)
            steps = pr_ready.plan(args(host="macmini"), "origin/main", code | {"tools/test_food_parity.py"}, packages,
                                  {"horizon-world"}, {})
            self.assertIn("--changed-files", steps[-1].command)

    def test_ci_s_rust_follows_the_six_week_train(self):
        self.assertEqual(pr_ready.ci_rust(datetime.date(2025, 2, 20)), 85)
        self.assertEqual(pr_ready.ci_rust(datetime.date(2025, 6, 26)), 88)
        self.assertEqual(pr_ready.ci_rust(datetime.date(2026, 10, 31)), 99)
        self.assertEqual(pr_ready.ci_rust(datetime.date(2026, 11, 12)), 100)

    def test_an_older_rust_warns_without_failing(self):
        today = datetime.date(2026, 10, 31)
        self.assertIsNone(pr_ready.old_rust((1, 99, 1), today))
        self.assertIsNone(pr_ready.old_rust(None, today))
        warning = pr_ready.old_rust((1, 97, 0), today)
        self.assertIn("1.97.0 is older than CI's stable (1.99)", warning)
        self.assertIn("`rustup update`", warning)
        steps = pr_ready.plan(args(quick=True), "origin/main", {"crates/horizon-core/src/jmath.rs"}, PACKAGES, LIBRARIES,
                              DEPENDENCIES)
        by_name = {step.name: step for step in steps}
        by_name["clippy"].status, by_name["clippy"].detail = "fail", "failed"
        pr_ready.mark_old_rust(steps, warning)
        names = [step.name for step in steps]
        self.assertEqual(names.index("Rust") + 1, names.index("clippy"))
        self.assertEqual(steps[names.index("Rust")].status, "warn")
        self.assertEqual(by_name["clippy"].detail, "failed; likely the old Rust above")
        steps = pr_ready.plan(args(quick=True), "origin/main", {"README.md"}, PACKAGES, LIBRARIES, DEPENDENCIES)
        pr_ready.mark_old_rust(steps, warning)
        self.assertNotIn("Rust", [step.name for step in steps])

    def test_summaries_of_check_output(self):
        output = "test result: ok. 3 passed; 0 failed;\ntest result: FAILED. 4 passed; 1 failed;\n"
        self.assertEqual(pr_ready.tests_passed(output), "7 passed, 1 failed")
        self.assertEqual(pr_ready.unittest_passed("....\nRan 12 tests in 1.0s\n\nOK\n"), "12 passed")
        self.assertEqual(pr_ready.unittest_passed("Ran 12 tests in 1.0s\n\nFAILED (failures=2)\n"),
                         "12 run, FAILED (failures=2)")
        parity_output = ("3 files changed: 2 harnesses depend on them, 2 without a pass for their current inputs.\n"
                         "\n1 passed, 1 failed in 2.0 min.\n  light: /x/.local/parity-runner/logs/light.log\n")
        self.assertEqual(pr_ready.parity_summary(parity_output), "2 affected, 1 passed, failed: light")
        self.assertEqual(pr_ready.parity_summary("2 files changed: 0 harnesses depend on them, 0 without a pass for "
                                                 "their current inputs.\nNothing to run: every harness passed."),
                         "no harness affected")
        self.assertEqual(pr_ready.duration(125), "2 min 5 s")

    def test_python_checks_run_beside_cargo_and_harnesses_after_it(self):
        steps = pr_ready.plan(args(), "origin/main", {"crates/horizon-core/src/lib.rs"}, PACKAGES, LIBRARIES,
                              DEPENDENCIES)
        steps[0].status = "pass"
        steps[-1].where, steps[-1].command, steps[-1].status = "here", "parity", None
        cargo_started = threading.Event()
        runs = []

        def run_script(group, host, verbose):
            names = [step.name for step in group]
            if "Python tests" in names:
                # Waits for cargo: running them first, alone, would time out here.
                runs.append(("python", cargo_started.wait(2)))
            elif "rustfmt" in names:
                cargo_started.set()
                runs.append(("cargo", host))
            elif names:
                runs.append(("after", names))

        with mock.patch.object(pr_ready, "run_script", run_script):
            pr_ready.run_checks(steps, "local", verbose=False)
        self.assertCountEqual(runs[:2], [("python", True), ("cargo", "local")])
        self.assertEqual(runs[2], ("after", ["Parity"]))

        runs.clear()
        cargo_started.clear()
        with mock.patch.object(pr_ready, "run_script", run_script):
            pr_ready.run_checks(steps, "local", verbose=True)
        self.assertEqual(runs[0], ("python", False))


if __name__ == "__main__":
    unittest.main()
