"""Dyna's switch routes only supported, same-repository events to its pool."""
import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time
import unittest

ROOT = Path(__file__).resolve().parents[1]
BASH = shutil.which("bash")
POSIX_BASH = BASH is not None and not (
    os.name == "nt" and os.path.dirname(os.path.normcase(BASH)).endswith(os.path.normcase(r"\system32")))


class DynaRoutingTests(unittest.TestCase):
    def route(self, event, same_repo="true", enabled="true", lease=None):
        env = dict(os.environ, EVENT=event, SAME_REPO=same_repo, ENABLED=enabled,
                   LEASE=str(int(time.time()) + 3600) if lease is None else lease)
        return subprocess.check_output(["bash", ROOT / "tools/ci/dyna_ci_runner.sh"], env=env, text=True).strip()

    @unittest.skipUnless(POSIX_BASH, "needs a bash that is not WSL's")
    def test_only_allowed_events_and_same_repository(self):
        for event in ("push", "workflow_dispatch", "pull_request"):
            self.assertEqual(self.route(event), "self_hosted=true", event)
        for event in ("pull_request_target", "workflow_run", "issue_comment", "schedule", ""):
            self.assertEqual(self.route(event), "self_hosted=false", event)
        self.assertEqual(self.route("pull_request", same_repo="false"), "self_hosted=false")

    @unittest.skipUnless(POSIX_BASH, "needs a bash that is not WSL's")
    def test_switch_and_lease_fail_closed(self):
        self.assertEqual(self.route("push", enabled="false"), "self_hosted=false")
        for lease in ("", "bad", "0"):
            self.assertEqual(self.route("push", lease=lease), "self_hosted=false")

    @unittest.skipUnless(POSIX_BASH, "needs a bash that is not WSL's")
    def test_live_and_snapshot_lease_routing_on_main(self):
        from test_ci_runners import jobs
        import textwrap
        route = dict(jobs((ROOT / ".github/workflows/dyna-ci.yml").read_text()))["changes"]
        script = textwrap.dedent(route.split('        run: |\n          if [ "$SAME_REPO"', 1)[1]
                                .split("      - id: hosted", 1)[0])
        script = 'if [ "$SAME_REPO" ' + script
        script = script.replace("${{ inputs.github-fallback }}", "false")
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "output"
            for live, snapshot, expected in (("9999999999", "0", "true"),
                                              ("", "9999999999", "true"),
                                              ("", "1794272194", "false"),
                                              ("0", "9999999999", "false")):
                stub = 'gh() { case "$*" in *SELF_HOSTED*) echo true;; *created_at*) date -u +%FT%TZ;; *) echo "$LIVE";; esac; };\n'
                env = dict(os.environ, EVENT="push", SAME_REPO="true", VARS_ENABLED="true",
                           VARS_LEASE=snapshot, LIVE=live, GITHUB_REPOSITORY="arismoko/horizon",
                           GITHUB_OUTPUT=str(output), GITHUB_RUN_ID="1")
                subprocess.run(["bash", "-e", "-c", stub + script], env=env, cwd=ROOT,
                               capture_output=True, text=True, check=True)
                self.assertIn(f"self_hosted={expected}\n", output.read_text())
                output.unlink()

    @unittest.skipUnless(POSIX_BASH, "needs a bash that is not WSL's")
    def test_snapshot_lease_is_bounded_by_run_age(self):
        # source, lease, created, now, expected: values independent of wall clock.
        cases = [
            ("live", "1100", "900", "1000", True),
            ("live", "950", "900", "1000", False),
            ("snapshot", "950", "900", "1000", True),
            ("snapshot", "899", "900", "1000", False),
            ("snapshot", "950", "900", "4500", True),
            ("snapshot", "950", "900", "4501", False),
            ("snapshot", "9999", "", "1000", False),
            ("snapshot", "9999", "bad", "1000", False),
            ("snapshot", "9999", "1001", "1000", False),
        ]
        for source, lease, created, now, expected in cases:
            with self.subTest(source=source, lease=lease, created=created, now=now):
                env = dict(os.environ, EVENT="push", SAME_REPO="true", ENABLED="true",
                           LEASE=lease, LEASE_SOURCE=source, CREATED=created, NOW=now)
                result = subprocess.check_output(["bash", ROOT / "tools/ci/dyna_ci_runner.sh"],
                                                 env=env, text=True)
                self.assertEqual(result.strip(), f"self_hosted={str(expected).lower()}")

    @unittest.skipIf(os.name == "nt", "the script installs dependencies on Dyna's Ubuntu runner")
    def test_cached_dependencies_export_persistent_paths(self):
        script = ROOT / "tools/ci/dyna_ci_deps.sh"
        with tempfile.TemporaryDirectory() as directory:
            cache = Path(directory)
            prefix = cache / "apt-prefix"
            prefix.mkdir()
            (prefix / ".dyna-hash").write_text(hashlib.sha256(script.read_bytes()).hexdigest() + "\n")
            envfile, pathfile = cache / "env", cache / "path"
            # Dyna sets CARGO_HOME and CARGO_TARGET_DIR on the runner, as in session boxes, so the
            # host's shared compiler cache matches; the script keeps them and only puts Cargo's bin on PATH.
            cargo_home = cache / "dyna-cargo"
            env = dict(os.environ, DYNA_CI_BUILD=directory, GITHUB_ENV=str(envfile), GITHUB_PATH=str(pathfile),
                       CARGO_HOME=str(cargo_home))
            subprocess.run(["bash", script], env=env, check=True)
            exports = envfile.read_text()
            self.assertIn(f"RUSTUP_HOME={cache / 'rustup'}\n", exports)
            for key in ("CARGO_TARGET_DIR", "CARGO_HOME", "CARGO_INCREMENTAL"):
                self.assertNotIn(key, exports)
            self.assertIn(str(cargo_home / "bin"), pathfile.read_text())

    @unittest.skipUnless(POSIX_BASH and os.name != "nt", "needs the runner's bash")
    def test_baked_packages_skip_the_download_and_a_partial_image_falls_back(self):
        script = ROOT / "tools/ci/dyna_ci_deps.sh"
        for missing, extraction_error in ((False, False), (True, False), (True, True)):
            with self.subTest(missing=missing, extraction_error=extraction_error), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                stubs = root / "stubs"
                stubs.mkdir()
                (stubs / "dpkg-query").write_text(
                    '#!/bin/bash\nif [ "$MISSING" = 1 ] && [ "${!#}" = libudev-dev ]; then exit 1; fi\nprintf installed\n')
                (stubs / "apt-get").write_text(
                    '#!/bin/bash\nprintf "%s\\n" "$*" >> "$APT_CALLS"\n'
                    'for arg in "$@"; do\n'
                    '  case "$arg" in Dir::Cache::archives=*) archives=${arg#*=} ;; esac\n'
                    'done\n'
                    'if [[ " $* " == *" install "* ]]; then touch "$archives/headers.deb"; fi\n')
                (stubs / "dpkg-deb").write_text(
                    '#!/bin/bash\nset -eu\n'
                    'test -f "$2"\n'
                    '[ "$EXTRACTION_ERROR" = 0 ] || exit 42\n'
                    'mkdir -p "$3/usr/include"\n'
                    'printf "fixture header\\n" > "$3/usr/include/fixture.h"\n'
                    'echo extracted >> "$EXTRACTION_RECEIPT"\n')
                for stub in stubs.iterdir():
                    stub.chmod(0o755)
                (root / "build").mkdir()
                env = dict(os.environ, PATH=f"{stubs}{os.pathsep}{os.environ['PATH']}", HOME=str(root / "home"),
                           DYNA_CI_BUILD=str(root / "build"), CARGO_HOME=str(root / "cargo"),
                           GITHUB_ENV=str(root / "env"), GITHUB_PATH=str(root / "path"),
                           MISSING=str(int(missing)), APT_CALLS=str(root / "apt-calls"), DYNA_CI_APT_TRIES="1",
                           EXTRACTION_ERROR=str(int(extraction_error)), EXTRACTION_RECEIPT=str(root / "extracted"))
                result = subprocess.run(["bash", script], env=env, capture_output=True, text=True)
                calls = (root / "apt-calls").read_text() if (root / "apt-calls").exists() else ""
                if missing:
                    self.assertTrue(calls, "a missing package must run the download fallback")
                    if extraction_error:
                        self.assertEqual(result.returncode, 42, result.stderr)
                        self.assertFalse((root / "env").exists())
                        self.assertFalse((root / "build/apt-prefix/.dyna-hash").exists())
                    else:
                        self.assertEqual(result.returncode, 0, result.stderr)
                        self.assertEqual((root / "extracted").read_text(), "extracted\n")
                        exports = dict(line.split("=", 1) for line in (root / "env").read_text().splitlines())
                        prefix = Path(exports["PKG_CONFIG_SYSROOT_DIR"])
                        self.assertEqual((prefix / "usr/include/fixture.h").read_text(), "fixture header\n")
                else:
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertEqual(calls, "")
                    self.assertIn("already installed", result.stdout)

    def run_deps_with_stub_apt(self, failures):
        """Run the script with an apt-get that fails `failures` times per command, then succeeds."""
        script = ROOT / "tools/ci/dyna_ci_deps.sh"
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            stubs = root / "stubs"
            stubs.mkdir()
            (stubs / "apt-get").write_text(
                '#!/usr/bin/env bash\n'
                f'n=$(cat "{root}/count" 2>/dev/null || echo 0); echo $((n + 1)) > "{root}/count"\n'
                f'if [ "$n" -lt {failures} ]; then echo "E: File has unexpected size (mirror sync)" >&2; exit 100; fi\n')
            (stubs / "dpkg-deb").write_text("#!/usr/bin/env bash\n")
            # No baked packages: the rootless download fallback is what these cases exercise.
            (stubs / "dpkg-query").write_text("#!/usr/bin/env bash\nexit 1\n")
            for stub in stubs.iterdir():
                stub.chmod(0o755)
            home = root / "home"
            home.mkdir()
            env = dict(os.environ, PATH=f"{stubs}{os.pathsep}{os.environ['PATH']}", HOME=str(home),
                       DYNA_CI_BUILD=str(root / "build"), CARGO_HOME=str(root / "cargo"),
                       GITHUB_ENV=str(root / "env"), GITHUB_PATH=str(root / "path"),
                       DYNA_CI_APT_PAUSE="0", DYNA_CI_APT_TRIES="3")
            (root / "build").mkdir()
            result = subprocess.run(["bash", script], env=env, capture_output=True, text=True)
            count = int((root / "count").read_text()) if (root / "count").exists() else 0
            return result, count

    @unittest.skipUnless(POSIX_BASH and os.name != "nt", "needs the runner's bash")
    def test_apt_is_retried_a_bounded_number_of_times_and_then_succeeds(self):
        # The first command (update) fails twice, then both commands pass: 2 failures + 2 successes.
        result, count = self.run_deps_with_stub_apt(failures=2)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(count, 4)
        self.assertEqual(result.stderr.count("retrying in"), 2)

    @unittest.skipUnless(POSIX_BASH and os.name != "nt", "needs the runner's bash")
    def test_apt_that_keeps_failing_fails_with_the_real_error_after_the_last_try(self):
        result, count = self.run_deps_with_stub_apt(failures=99)
        self.assertEqual(result.returncode, 100)
        self.assertEqual(count, 3)
        self.assertEqual(result.stderr.count("E: File has unexpected size"), 3)
        self.assertIn("timeout failed after 3 tries (exit 100)", result.stderr)

    def test_lint_enforces_api_boundary_under_central_selection(self):
        from test_ci_runners import jobs
        lint = dict(jobs((ROOT / ".github/workflows/dyna-ci.yml").read_text()))["lint"]
        self.assertIn("if: ${{ always() && !cancelled()", lint)
        # Pin the executable contract, including the selection and source-only mode:
        # deleting the step or making it unconditional must fail this test.
        step = lint.split("- name: Check mod API boundary\n", 1)[1].split("\n      - ", 1)[0]
        self.assertIn("needs.changes.outputs.rust_required != 'false'", step)
        self.assertIn("python3 tools/ci/check_api_boundary.py --source-only", step)

    def test_workflow_wires_route_and_keeps_privileges_off_pool(self):
        from test_ci_runners import jobs
        workflow = (ROOT / ".github/workflows/dyna-ci.yml").read_text()
        blocks = dict(jobs(workflow))
        route = blocks["changes"]
        for item in ("vars.DYNA_CI_SELF_HOSTED", "vars.DYNA_CI_POOL_UNTIL", "github.event_name",
                     "github.event.pull_request.head.repo.full_name == github.repository", "dyna_ci_runner.sh"):
            self.assertIn(item, route)
        for name, label in (("verify", "dyna-large"), ("verify-static", "dyna-large"), ("launcher", "dyna-medium")):
            block = blocks[name]
            self.assertIn("needs: changes", block)
            self.assertIn(f"needs.changes.outputs.self_hosted == 'true' && '{label}'", block)
            self.assertNotIn("secrets.", block)
            self.assertNotIn("permissions:", block)
        self.assertIn("runs-on: ubuntu-latest", blocks["pool-watch"])
        self.assertIn("permissions:\n  contents: read", workflow)

    def test_split_jobs_are_independent_fail_closed_and_share_routing(self):
        from test_ci_runners import jobs
        blocks = dict(jobs((ROOT / ".github/workflows/dyna-ci.yml").read_text()))
        runtime = blocks["verify"]
        static = blocks["verify-static"]
        # verify waits for the heavy gate (tools/ci/pr_gate.py); verify-static runs on every pull request.
        conditions = {
            id(runtime): "${{ always() && !cancelled() && needs.changes.result != 'skipped' && "
                         "(needs.changes.result != 'success' || needs.changes.outputs.heavy == 'true') }}",
            id(static): "${{ always() && !cancelled() && needs.changes.result != 'skipped' }}",
        }
        for block in (runtime, static):
            header, steps = block.split("    steps:\n", 1)
            fields = dict(line.strip().split(":", 1) for line in header.splitlines()
                          if line.startswith("    ") and ":" in line and not line.lstrip().startswith("#"))
            self.assertEqual(fields["needs"].strip(), "changes")
            self.assertIn("timeout-minutes", fields)
            self.assertEqual(fields["if"].strip(), conditions[id(block)])
            first = steps.split("\n      - ", 1)[0]
            guard = dict(line.strip().split(":", 1) for line in first.splitlines()
                         if line.startswith("        ") and ":" in line)
            self.assertEqual(guard["if"].strip(), "needs.changes.result != 'success'")
            self.assertEqual(guard["run"].strip(), "exit 1")
        # verify-static routes as verify does, except that a contributor's pull request stays on GitHub's runners.
        for field, fork in (("runs-on", "needs.changes.outputs.trusted != 'true' && 'ubuntu-latest' || "),
                            ("RUNNER_KIND", "needs.changes.outputs.trusted != 'true' && 'github' || ")):
            shared = runtime.split(f"{field}:", 1)[1].split("\n", 1)[0]
            if field == "runs-on":
                shared = shared.replace("'ubuntu-latest' || ", "'ubuntu-latest' || " + fork, 1)
            else:
                shared = shared.replace("${{ ", "${{ " + fork, 1)
            self.assertEqual(static.split(f"{field}:", 1)[1].split("\n", 1)[0], shared)
        self.assertIn("- name: No Rust work selected", runtime)
        self.assertNotIn("--unshallow", runtime)
        self.assertIn("--unshallow", static)
        self.assertNotIn("cargo test", static)
        self.assertNotIn("Swatinem/rust-cache", static)
        self.assertNotIn("runner-packages.txt", static)
        self.assertIn("python3 tools/ci/ignored_shards.py --prepare horizon-mod-package $JOBS", runtime)
        self.assertIn("cc -shared -fPIC -O2 -Wall -Wextra -Werror tools/ci/network_guard.c -o target/network_guard.so", runtime)
        self.assertIn("python3 tools/ci/run_test_binaries.py --jobs 3 --time-limits 2,5 --network-guard target/network_guard.so -- cargo test $JOBS --profile test --workspace --lib --bins --tests", runtime)
        self.assertIn("cargo test $JOBS --no-fail-fast --profile test --workspace --doc", runtime)
        tools = static.split("- name: Install Rust tools for isolated tooling fixtures\n", 1)[1].split("\n      - ", 1)[0]
        self.assertIn("needs.changes.outputs.rust_required != 'false' || needs.changes.outputs.tooling_required != 'false'", tools)
        self.assertIn("--component rustfmt --component clippy", tools)
        # GITHUB_PATH, not a shell-local export, carries Cargo into later steps.
        self.assertIn('echo "$CARGO_HOME/bin" >> "$GITHUB_PATH"', tools)
        self.assertLess(static.index("- name: Install Rust tools for isolated tooling fixtures"),
                        static.index("- name: Python tests of the repository's tools"))

    @unittest.skipUnless(POSIX_BASH and os.name != "nt", "needs the runner's bash and POSIX paths")
    def test_verify_reuses_persistent_target_without_replacing_existing_paths(self):
        import textwrap
        from test_ci_runners import jobs
        runtime = dict(jobs((ROOT / ".github/workflows/dyna-ci.yml").read_text()))["verify"]
        setup = runtime.split("- name: Use persistent verify build artifacts\n", 1)[1].split("\n      - ", 1)[0]
        self.assertIn("needs.changes.outputs.rust_required != 'false' && needs.changes.outputs.self_hosted == 'true'", setup)
        script = textwrap.dedent(setup.split("        run: |\n", 1)[1])
        restore = runtime.split("- name: Restore unchanged source timestamps\n", 1)[1].split("\n      - ", 1)[0]
        self.assertIn("needs.changes.outputs.rust_required != 'false'", restore)
        self.assertIn("python3 tools/ci/source_mtimes.py target/ci-source-mtimes-v2.json", restore)
        self.assertLess(runtime.index("- name: Use persistent verify build artifacts"),
                        runtime.index("- name: Restore unchanged source timestamps"))
        self.assertLess(runtime.index("- name: Restore unchanged source timestamps"),
                        runtime.index("- name: Build the runtime holder test actor"))
        for case in ("absent", "correct-link", "directory", "file", "wrong-link", "dangling-link"):
            with self.subTest(case=case), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                persistent = root / "persistent"
                persistent.mkdir()
                target = root / "target"
                other = root / "other"
                other.mkdir()
                if case == "correct-link":
                    target.symlink_to(persistent)
                elif case == "directory":
                    target.mkdir()
                    (target / "keep").write_text("retained")
                elif case == "file":
                    target.write_text("retained")
                elif case in ("wrong-link", "dangling-link"):
                    target.symlink_to(other if case == "wrong-link" else root / "missing")
                before = target.lstat() if case != "absent" else None
                before_link = os.readlink(target) if case in ("wrong-link", "dangling-link") else None
                env = dict(os.environ, CARGO_TARGET_DIR=str(persistent))
                result = subprocess.run(["bash", "-e", "-c", script], cwd=root, env=env,
                                        capture_output=True, text=True)
                if case in ("absent", "correct-link"):
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertEqual(target.resolve(), persistent)
                else:
                    self.assertNotEqual(result.returncode, 0)
                    after = target.lstat()
                    # Following a symlink may update atime without modifying it.
                    fields = ("st_ino", "st_dev", "st_mode", "st_size", "st_mtime_ns")
                    self.assertEqual(tuple(getattr(after, field) for field in fields),
                                     tuple(getattr(before, field) for field in fields))
                    if case in ("wrong-link", "dangling-link"):
                        self.assertEqual(os.readlink(target), before_link)
                    if case == "directory":
                        self.assertEqual((target / "keep").read_text(), "retained")
                    elif case == "file":
                        self.assertEqual(target.read_text(), "retained")

    @unittest.skipUnless(POSIX_BASH and os.name != "nt", "needs the runner's bash and POSIX paths")
    def test_stage_artifact_handoff_selects_normal_binary_and_rejects_unsafe_paths(self):
        import json
        import textwrap
        from test_ci_runners import jobs
        runtime = dict(jobs((ROOT / ".github/workflows/dyna-ci.yml").read_text()))["verify"]
        build = runtime.split("- name: Build the source stage generator (test profile)\n", 1)[1].split("\n      - ", 1)[0]
        script = textwrap.dedent(build.split("        run: |\n", 1)[1])
        for case in ("normal-and-harness", "harness-only", "newline-path"):
            with self.subTest(case=case), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                stubs = root / "bin"
                stubs.mkdir()
                cargo = stubs / "cargo"
                cargo.write_text("#!/usr/bin/env python3\nimport os, sys\nsys.stdout.write(os.environ['STAGE_ARTIFACTS'])\n")
                cargo.chmod(0o755)
                normal = root / ("line\nbreak" if case == "newline-path" else "horizon-stage")
                normal.touch()
                harness = root / "horizon-stage-harness"
                harness.touch()
                messages = [{"reason": "compiler-artifact", "target": {"name": "horizon-stage", "kind": ["bin"]},
                             "profile": {"test": True}, "executable": str(harness)}]
                if case != "harness-only":
                    messages.append({"reason": "compiler-artifact", "target": {"name": "horizon-stage", "kind": ["bin"]},
                                     "profile": {"test": False}, "executable": str(normal)})
                environment = root / "github-env"
                env = dict(os.environ, PATH=f"{stubs}{os.pathsep}{os.environ['PATH']}",
                           RUNNER_TEMP=str(root), GITHUB_ENV=str(environment), JOBS="",
                           STAGE_ARTIFACTS="".join(json.dumps(message) + "\n" for message in messages))
                result = subprocess.run(["bash", "-e", "-c", script], cwd=ROOT, env=env,
                                        capture_output=True, text=True)
                if case == "normal-and-harness":
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertEqual(environment.read_text(), f"HORIZON_STAGE_TOOL={normal.resolve()}\n")
                else:
                    self.assertNotEqual(result.returncode, 0)
                    self.assertFalse(environment.exists())

    def assert_positive_stage_export(self, verify):
        command = "python3 tools/ci/test_stage_catalog.py -v"
        name = "- name: Validate prebuilt source stage and listed slots\n"
        step = verify.split(name, 1)[1].split("\n      - ", 1)[0]
        # Inspect active scalar values in this named step, not command text in
        # comments or another step. Require the exact executable invocation.
        fields = dict(line.strip().split(":", 1) for line in step.splitlines()
                      if line.startswith("        ") and ":" in line and not line.lstrip().startswith("#"))
        self.assertEqual(fields.get("run", "").strip(), command)
        self.assertIn("steps.stage.outcome == 'success'", fields.get("if", ""))
        self.assertEqual(fields.get("continue-on-error", "false").strip(), "false")
        self.assertEqual(verify.count(command), 1)
        self.assertLess(verify.index(name), verify.index("- name: Test Rust documentation examples"))
        self.assertNotIn("python3 tools/ci/stage_slots.py", verify)
        self.assertLess(verify.index("- name: Install Rust toolchain and the runner image's packages"), verify.index(name))
        self.assertLess(verify.index("uses: actions/setup-python@v5"), verify.index(name))
        build = verify.split("- name: Build the source stage generator (test profile)\n", 1)[1].split("\n      - ", 1)[0]
        self.assertIn("cargo build $JOBS --locked --profile test -p horizon-mod-api-gen --bin horizon-stage", build)
        self.assertIn("HORIZON_STAGE_TOOL=", build)
        self.assertLess(verify.index("- name: Build the source stage generator (test profile)"), verify.index(name))

    def test_positive_stage_export_runs_once_before_docs_outside_offline_discovery(self):
        from test_ci_runners import jobs
        verify = dict(jobs((ROOT / ".github/workflows/dyna-ci.yml").read_text()))["verify"]
        self.assert_positive_stage_export(verify)

    def test_the_surface_report_runs_after_the_stage_build_on_pull_requests_and_dispatches(self):
        from test_ci_runners import jobs
        verify = dict(jobs((ROOT / ".github/workflows/dyna-ci.yml").read_text()))["verify"]
        name = "- name: Report the mod API surface change\n"
        self.assertEqual(verify.count(name), 1)
        step = verify.split(name, 1)[1].split("\n      - ", 1)[0]
        fields = {}
        for line in step.splitlines():
            if ":" in line and not line.lstrip().startswith("#"):
                key, value = line.strip().split(":", 1)
                fields[key] = value.strip()
        self.assertEqual(fields.get("run"), "python3 tools/ci/surface_report.py")
        self.assertIn("steps.stage.outcome == 'success'", fields.get("if", ""))
        self.assertIn("github.event_name == 'pull_request' || inputs.pr != ''", fields.get("if", ""))
        self.assertEqual(fields.get("EVENT"), "${{ github.event_name }}")
        self.assertEqual(fields.get("PR"), "${{ inputs.pr }}")
        self.assertEqual(fields.get("GH_TOKEN"), "${{ github.token }}")
        self.assertNotIn("continue-on-error", fields)
        # It needs the tool the stage build puts in HORIZON_STAGE_TOOL.
        self.assertLess(verify.index("- name: Build the source stage generator (test profile)"), verify.index(name))

    def test_positive_stage_guard_rejects_commented_command_noop_and_ignored_failure(self):
        from test_ci_runners import jobs
        verify = dict(jobs((ROOT / ".github/workflows/dyna-ci.yml").read_text()))["verify"]
        command = "        run: python3 tools/ci/test_stage_catalog.py -v"
        for replacement in (
            "        # run: python3 tools/ci/test_stage_catalog.py -v\n        run: true",
            "        run: '# python3 tools/ci/test_stage_catalog.py -v'",
            command + "\n        continue-on-error: true",
        ):
            with self.subTest(replacement=replacement), self.assertRaises(AssertionError):
                self.assert_positive_stage_export(verify.replace(command, replacement, 1))

    def test_optional_map_upload_is_nonblocking_and_reports_its_failure(self):
        from test_ci_runners import jobs
        verify = dict(jobs((ROOT / ".github/workflows/dyna-ci.yml").read_text()))["verify-static"]
        upload = verify.split("        id: mod_host_map_upload\n", 1)[1].split("\n      - ", 1)[0]
        fields = dict(line.strip().split(":", 1) for line in upload.splitlines()
                      if line.startswith("        ") and ":" in line and not line.lstrip().startswith("#"))
        self.assertEqual(fields["continue-on-error"].strip(), "true")
        self.assertIn("uses: actions/upload-artifact@", upload)
        report = verify.split("- name: Report optional mod-host map upload failure\n", 1)[1].split("\n      - ", 1)[0]
        condition = report.split("if:", 1)[1].split("\n", 1)[0]
        self.assertIn("!cancelled()", condition)
        self.assertIn("steps.mod_host_map_upload.outcome == 'failure'", condition)
        self.assertIn("::warning::", report)
        self.assertIn('>> "$GITHUB_STEP_SUMMARY"', report)

    def test_tooling_tests_run_only_after_static_setup_succeeded(self):
        # Failed setup must not produce secondary failures from compiler fixtures (#2920).
        from test_ci_runners import jobs
        verify = dict(jobs((ROOT / ".github/workflows/dyna-ci.yml").read_text()))["verify-static"]
        step = verify.split("- name: Python tests of the repository's tools\n", 1)[1].split("\n      - ", 1)[0]
        condition = step.split("if:", 1)[1].split("\n", 1)[0]
        self.assertNotIn("success()", condition)
        self.assertIn("!cancelled()", condition)
        self.assertIn("steps.setup.outcome == 'success'", condition)
        self.assertNotIn("always()", condition)

    @unittest.skipUnless(POSIX_BASH, "needs a bash that is not WSL's")
    def test_missing_prepared_data_fails_the_leg(self):
        from test_ci_runners import jobs
        import textwrap
        workflow = (ROOT / ".github/workflows/ignored-tests.yml").read_text()
        shard = dict(jobs(workflow))["ignored-tests-shards"]
        script = textwrap.dedent(shard.split("        run: |\n          for dir", 1)[1]
                                .split("      - name: Generate", 1)[0])
        script = "for dir " + script
        with tempfile.TemporaryDirectory() as directory:
            script = script.replace("/cache/horizon-local", directory + "/missing")
            summary = Path(directory) / "summary"
            for event in ("push", "workflow_dispatch"):
                env = dict(os.environ, EVENT=event, GITHUB_STEP_SUMMARY=str(summary))
                run = subprocess.run(["bash", "-e", "-c", script], env=env,
                                     capture_output=True, text=True)
                self.assertEqual(run.returncode, 1)
                self.assertIn(f"Prepared data unavailable: minecraft ({event})", summary.read_text())
                summary.unlink()

    def test_telemetry_ingest_checks_require_manual_dispatch(self):
        from test_ci_runners import triggers
        workflow = (ROOT / ".github/workflows/telemetry-server.yml").read_text()
        self.assertEqual(triggers(workflow), {"workflow_dispatch"})

    def test_schedule_only_dispatches_and_parity_only_runs_on_dispatch(self):
        from test_ci_runners import jobs, triggers
        workflow = (ROOT / ".github/workflows/dyna-ci.yml").read_text()
        self.assertEqual(triggers(workflow), {"pull_request", "push", "workflow_dispatch", "schedule"})
        blocks = dict(jobs(workflow))
        dispatcher = blocks["nightly-dispatch"]
        self.assertIn("github.event_name == 'schedule'", dispatcher)
        self.assertIn("runs-on: ubuntu-latest", dispatcher)
        self.assertIn('gh workflow run dyna-ci.yml --repo "$GITHUB_REPOSITORY" --ref main', dispatcher)
        self.assertIn("if: github.event_name != 'schedule'", blocks["changes"])
        parity = blocks["vanilla-parity"]
        self.assertIn("github.event_name == 'workflow_dispatch'", parity)
        self.assertIn("github.repository == 'arismoko/horizon'", parity)
        self.assertIn("runs-on: dyna-large", parity)
        self.assertIn("persist-credentials: false", parity)


if __name__ == "__main__":
    unittest.main()
