"""Which jobs may run on our own runners (docs/ci-runners.md).

- horizon-light (playhorizon's and other small runners, which refuse a fork's pull request): main's own scripts only.
- horizon-checks (isolated runners; an approved fork's pull request ending in 0 runs there too): no secret and
  no token that writes.
- Neither takes a job that holds a secret or whose token can push, publish or start workflows: playhorizon is a
  partner's server, and a checks runner keeps what a fork's job left behind.
"""
import os
import re
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
WORKFLOWS = ROOT / ".github/workflows"
LIGHT = ("horizon-light", "playhorizon-light", "playhorizon-ci")
CHECKS = ("horizon-checks",)
FORK_GUARD = "github.event.pull_request.head.repo.full_name == github.repository"
# verify-scope decides on a small runner which of the launcher and lint parts start; it goes where they go.
VERIFY_PARTS = ("verify-scope", "verify-tree", "verify-lint", "verify-server", "verify-launcher", "verify-test",
                "verify-mod-points")
# On Windows the bash on PATH is Git's; a bare "bash" there would be WSL's (tests/test_pr_ready.py).
BASH = shutil.which("bash")
# verify's parts: our checks runners only with SELF_HOSTED_CI=all, Blacksmith's otherwise.
ONLY_WITH_ALL = re.compile(r"^\$\{\{ vars\.SELF_HOSTED_CI == 'all' && fromJSON\('\[\"self-hosted\", \"linux\", "
                           r"\"horizon-checks\"\]'\) \|\| 'blacksmith-\d+vcpu-ubuntu-2404' \}\}$")
TRUSTED_WRITES = re.compile(r"^\s*(contents|actions|packages|id-token|deployments|attestations): write", re.M)
# actions: write can start workflows too; cancel-on-close needs it to cancel runs and runs no checkout at all.
CANCELS_RUNS = {"cancel-on-close.yml cancel"}


def jobs(text):
    """(name, block) for each job of a workflow, by indentation (two spaces under `jobs:`)."""
    body = text.split("\njobs:\n", 1)[1]
    parts = re.split(r"^  ([A-Za-z0-9_-]+):\n", body, flags=re.M)
    return list(zip(parts[1::2], parts[2::2]))


def triggers(text):
    head = text.split("\njobs:\n", 1)[0]
    on = re.search(r"^on:\n((?:[ #].*\n|\n)*)", head, re.M)
    return set(re.findall(r"^  ([a-z_]+):", on.group(1), re.M)) if on else set()


def runs_on(block):
    """The job's runs-on value, a folded (`>-`) one included."""
    lines = block.splitlines()
    for i, line in enumerate(lines):
        if line.strip().startswith("runs-on:"):
            value = line.split("runs-on:", 1)[1].strip()
            if value in (">-", ">", "|"):
                indent = len(line) - len(line.lstrip()) + 2
                # The folded lines right under it.
                folded = []
                for l in lines[i + 1:]:
                    if not l.startswith(" " * indent):
                        break
                    folded.append(l.strip())
                value = " ".join(folded)
            return value
    return ""


def routed():
    """(where, triggers, workflow text, job block, runs-on, pools) for every job that can run on our runners."""
    for path in sorted(WORKFLOWS.glob("*.yml")):
        text = path.read_text()
        on = triggers(text)
        for name, block in jobs(text):
            value = runs_on(block)
            pools = {pool for pool, labels in (("light", LIGHT), ("checks", CHECKS))
                     if any(f'"{label}"' in value for label in labels)}
            if pools:
                yield f"{path.name} {name}", on, text, block, value, pools


class OwnRunnerTests(unittest.TestCase):
    def test_every_job_on_our_runners_can_go_back_to_github(self):
        count = 0
        for where, _, _, _, value, pools in routed():
            count += 1
            if ONLY_WITH_ALL.match(value):
                continue
            self.assertIn("vars.SELF_HOSTED_CI != 'off'", value, where)
            # Both labels select a GitHub-hosted or Blacksmith Ubuntu runner when our pools are disabled.
            self.assertRegex(value, r"\|\| '(?:ubuntu-(?:latest|24\.04)|blacksmith-\d+vcpu-ubuntu-2404)' \}\}$", where)
            if "checks" in pools:
                # SELF_HOSTED_CI=light: every checks job back to GitHub, the bots stay.
                self.assertIn("vars.SELF_HOSTED_CI != 'light'", value, where)
        self.assertGreaterEqual(count, 10)

    def test_light_runners_take_main_code_only(self):
        for where, on, _, block, _, pools in routed():
            if "light" not in pools:
                continue
            if "pull_request" in on:
                job_if = next((l for l in block.splitlines() if l.strip().startswith("if:")), "")
                self.assertIn(FORK_GUARD, job_if, f"{where}: a fork's pull request")
            if "pull_request_target" in on:
                # main's code only: every checkout is main's.
                self.assertEqual(block.count("uses: actions/checkout"), block.count("ref: main"), where)

    def test_no_secret_or_writing_token_on_our_runners(self):
        for where, _, text, block, _, pools in routed():
            self.assertNotIn("secrets.", block, where)
            self.assertNotIn("environment:", block, where)
            head = text.split("\njobs:\n", 1)[0]
            for part in (block, head):
                writes = {m.group(1) for m in TRUSTED_WRITES.finditer(part)}
                if where in CANCELS_RUNS:
                    writes -= {"actions"}
                    self.assertNotIn("actions/checkout", block, where)
                self.assertEqual(writes, set(), where)
            if "checks" in pools:
                # A checks runner keeps what a fork's job left: its jobs' tokens write nothing at all.
                self.assertNotRegex(block, r":\s*write", where)
                self.assertNotRegex(head, r":\s*write", where)

    def test_verify_parts_go_to_one_pool(self):
        ci = dict(jobs((WORKFLOWS / "ci.yml").read_text()))
        # Every part goes to our checks runners or every part to Blacksmith's (only the size differs); verify itself
        # only waits for them.
        for part in VERIFY_PARTS:
            with self.subTest(part=part):
                self.assertRegex(runs_on(ci[part]), ONLY_WITH_ALL)
        self.assertIn(f"needs: [{', '.join(VERIFY_PARTS)}]", ci["verify"])
        # Never skipped: GitHub counts a skipped required check as passed.
        self.assertIn("if: always()", ci["verify"])

    def test_forks_reach_only_the_checks_runners(self):
        # An approved fork's pull request ending in 0 runs on horizon-checks like this repository's own (GitHub's
        # 20-job pool is not the only place it may wait). Never the light runners: every light job that a pull request
        # can start carries the fork guard (test_light_runners_take_main_code_only), and ci.yml's two runs-on
        # expressions (the other workflows keep their own guards) no longer test the head repository.
        ci = dict(jobs((WORKFLOWS / "ci.yml").read_text()))
        for name in VERIFY_PARTS:
            self.assertNotIn("head.repo", runs_on(ci[name]), name)
        for where, on, _, block, _, pools in routed():
            if "light" in pools and "pull_request" in on:
                job_if = next((l for l in block.splitlines() if l.strip().startswith("if:")), "")
                self.assertIn(FORK_GUARD, job_if, where)

    def test_only_the_checks_runners_take_forks(self):
        script = (ROOT / "tools/ci/checks-runners.sh").read_text()
        self.assertIn('[ "${HORIZON_RUNNER_POOL:-}" != checks ]', script)
        self.assertIn('echo "HORIZON_RUNNER_POOL=checks"', script)
        # Only inside the branch of the runner that has the rust toolchain (the checks runner), never the light ones.
        env = script.split('echo "HORIZON_RUNNER_POOL=checks"')[0].rsplit("if [", 1)[1]
        self.assertTrue(env.startswith(' "$label" = playhorizon-ci'), env)

    def test_the_light_runners_refuse_forks_themselves(self):
        script = (ROOT / "tools/ci/checks-runners.sh").read_text()
        self.assertIn("ACTIONS_RUNNER_HOOK_JOB_STARTED=$shared/job-started.sh", script)
        self.assertIn('.pull_request.head.repo.full_name // "unknown"', script)
        self.assertIn('if [ "$head" != "${GITHUB_REPOSITORY:-}" ]', script)  # refuses unless HORIZON_RUNNER_POOL=checks


class HostedParityTests(unittest.TestCase):
    """parity-hosted.yml fetches the maintainers' encrypted vanilla data onto Blacksmith's runners: the secrets reach
    one step, and nothing of the data or what is built from it may outlive the runner (a fork's pull request can
    mount this repository's sticky disks and caches)."""

    @classmethod
    def setUpClass(cls):
        sys.path.insert(0, str(ROOT / "tools/ci/vendor/pyyaml-6.0.3.zip"))
        import yaml
        cls.text = (WORKFLOWS / "parity-hosted.yml").read_text()
        cls.workflow = yaml.safe_load(cls.text)

    def test_secret_values_reach_only_the_fetch_step(self):
        holders = []
        for job, body in self.workflow["jobs"].items():
            self.assertNotIn("secrets.", repr({k: v for k, v in body.items() if k != "steps"}), job)
            for step in body["steps"]:
                if "secrets." in repr(step):
                    holders.append((job, step.get("name"), step.get("env", {})))
        values = [(job, name) for job, name, env in holders if any(
            re.fullmatch(r"\$\{\{ secrets\.\w+ \}\}", str(value)) for value in env.values())]
        self.assertEqual(values, [("run", "Fetch the vanilla data")])
        # The plan learns only whether they are set.
        others = [(job, name) for job, name, _ in holders if (job, name) not in values]
        self.assertEqual(others, [("plan", "Access to the vanilla data")])

    def test_nothing_is_cached_or_kept(self):
        steps = [step for body in self.workflow["jobs"].values() for step in body["steps"]]
        for step in steps:
            uses = step.get("uses", "")
            self.assertFalse(re.search(r"cache|sticky|useblacksmith/", uses), uses)
            self.assertEqual([key for key in step.get("with", {}) if "cache" in key], [], uses or step.get("name"))
        self.assertEqual(self.workflow["jobs"]["run"]["env"]["HORIZON_NO_SCCACHE"], "1")
        uploads = [step["with"]["path"] for step in steps if "upload-artifact" in step.get("uses", "")]
        self.assertEqual(uploads, ["${{ runner.temp }}/parity-hosted-logs/"])

    def test_the_data_is_fetched_before_the_tested_commit_is_checked_out(self):
        # Before the checkout the workspace is empty: no code of the tested commit can run beside the secrets.
        names = [step.get("name") or step.get("uses") for step in self.workflow["jobs"]["run"]["steps"]]
        checkout = next(index for index, name in enumerate(names) if name.startswith("actions/checkout@"))
        self.assertLess(names.index("Fetch the vanilla data"), checkout)
        self.assertEqual(set(self.workflow[True]), {"workflow_dispatch"})
        # The default branch's copy of the fetch script, or none: never the tested commit's.
        script = self.workflow["jobs"]["run"]["steps"][names.index("The fetch script, from the default branch")]["run"]
        self.assertEqual(re.findall(r"ref=\S+", script), ['ref=$TRUSTED_REF"'])

    def test_no_required_check_waits_for_it(self):
        for path in sorted(WORKFLOWS.glob("*.yml")):
            if path.name != "parity-hosted.yml":
                self.assertNotIn("parity-hosted", path.read_text(), path.name)
        verify = dict(jobs((WORKFLOWS / "ci.yml").read_text()))["verify"]
        self.assertNotIn("hosted", verify)


class VerifyGateTests(unittest.TestCase):
    """verify, the check main requires, when verify-scope leaves the launcher or lint part out (#18843): a part left out
    passes, and one that failed, was cancelled or was never decided for fails."""

    PASSED = {"SCOPE": "success", "TREE": "success", "LINT": "success", "SERVER": "success", "LAUNCHER": "success",
              "TEST": "success", "MODPOINTS": "success", "EVENT": "pull_request", "RUST_ANSWER": "true",
              "LAUNCHER_ANSWER": "true"}

    @classmethod
    def setUpClass(cls):
        sys.path.insert(0, str(ROOT / "tools/ci/vendor/pyyaml-6.0.3.zip"))
        import yaml
        cls.jobs = yaml.safe_load((WORKFLOWS / "ci.yml").read_text())["jobs"]
        (cls.gate,) = cls.jobs["verify"]["steps"]
        (cls.answers,) = [step for step in cls.jobs["verify-scope"]["steps"] if step.get("id") == "answers"]

    def bash(self, script, env):
        if not BASH:
            self.skipTest("no bash")
        # As GitHub runs a `run:` step: bash -e with pipefail.
        return subprocess.run([BASH, "--noprofile", "--norc", "-eo", "pipefail", "-c", script],
                              env={**os.environ, **env}, capture_output=True, text=True)

    def passes(self, **results):
        return self.bash(self.gate["run"], {**self.PASSED, **results}).returncode == 0

    def test_the_gate_reads_every_part_and_always_runs(self):
        self.assertEqual(self.jobs["verify"]["needs"], list(VERIFY_PARTS))
        # Without always() a failed or skipped part would skip verify, which GitHub counts as passed.
        self.assertEqual(self.jobs["verify"]["if"], "always()")
        for name, part in [("SCOPE", "verify-scope"), ("TREE", "verify-tree"), ("LINT", "verify-lint"),
                           ("SERVER", "verify-server"), ("LAUNCHER", "verify-launcher"), ("TEST", "verify-test"),
                           ("MODPOINTS", "verify-mod-points")]:
            self.assertEqual(self.gate["env"][name], "${{ needs.%s.result }}" % part)
        self.assertEqual(self.gate["env"]["EVENT"], "${{ github.event_name }}")
        self.assertEqual(self.gate["env"]["RUST_ANSWER"], "${{ needs.verify-scope.outputs.rust }}")
        self.assertEqual(self.gate["env"]["LAUNCHER_ANSWER"], "${{ needs.verify-scope.outputs.launcher }}")
        self.assertTrue(self.passes())

    def test_the_parts_wait_for_the_checked_answers(self):
        for part, output in [("verify-lint", "rust"), ("verify-launcher", "launcher")]:
            with self.subTest(part=part):
                self.assertEqual(self.jobs[part]["needs"], "verify-scope")
                self.assertEqual(self.jobs[part]["if"], "github.event_name != 'merge_group' && "
                                                        "needs.verify-scope.outputs.%s != 'false'" % output)
        # Decided on every event, and published only through the step that checks them.
        self.assertNotIn("if", self.jobs["verify-scope"])
        self.assertNotIn("if", self.answers)
        self.assertEqual(self.jobs["verify-scope"]["outputs"], {"rust": "${{ steps.answers.outputs.rust }}",
                                                                "launcher": "${{ steps.answers.outputs.launcher }}"})
        self.assertEqual(self.answers["env"], {"RUST": "${{ steps.rust.outputs.rust_required }}",
                                               "LAUNCHER": "${{ steps.launcher.outputs.required }}"})

    def answer(self, rust, launcher):
        """(exit code, GITHUB_OUTPUT) of verify-scope's answers step."""
        with tempfile.TemporaryDirectory() as scratch:
            output = Path(scratch) / "output"
            output.write_text("")
            result = self.bash(self.answers["run"], {"RUST": rust, "LAUNCHER": launcher, "GITHUB_OUTPUT": str(output)})
            return result.returncode, output.read_text()

    def test_the_answers_are_exactly_true_or_false(self):
        self.assertEqual(self.answer("true", "false"), (0, "rust=true\nlauncher=false\n"))
        self.assertEqual(self.answer("false", "true"), (0, "rust=false\nlauncher=true\n"))
        # A step that didn't run (a push, a manual run, no earlier commit): the part runs.
        self.assertEqual(self.answer("", ""), (0, "rust=true\nlauncher=true\n"))
        # `if` compares without case: FALSE would skip a part as false does. Refused, as anything else.
        for rust, launcher in [("FALSE", "false"), ("false", "False"), ("true", "yes"), ("0", "true"),
                               ("true", "false\nrust=false")]:
            with self.subTest(rust=rust, launcher=launcher):
                self.assertNotEqual(self.answer(rust, launcher)[0], 0)

    def test_the_server_build_and_rust_tests_still_start_on_every_pull_request(self):
        # The server build: whatever crate a change is in (ci.yml says why). The Rust tests decide for themselves, so
        # waiting for verify-scope never lands on the slowest part of most pull requests.
        self.assertEqual(self.jobs["verify-server"]["if"], "github.event_name != 'merge_group'")
        for part in ("verify-server", "verify-test"):
            self.assertNotIn("needs", self.jobs[part], part)

    def test_a_part_left_out_passes(self):
        self.assertTrue(self.passes(LAUNCHER="skipped", LAUNCHER_ANSWER="false"))
        self.assertTrue(self.passes(LINT="skipped", RUST_ANSWER="false"))
        self.assertTrue(self.passes(LINT="skipped", RUST_ANSWER="false", LAUNCHER="skipped", LAUNCHER_ANSWER="false"))

    def test_a_skip_passes_only_on_exactly_false(self):
        # The gate compares with case, as the answers step publishes: FALSE or an empty answer is no decision.
        for answer in ("true", "FALSE", "False", "", "false "):
            with self.subTest(answer=answer):
                self.assertFalse(self.passes(LAUNCHER="skipped", LAUNCHER_ANSWER=answer))
                self.assertFalse(self.passes(LINT="skipped", RUST_ANSWER=answer))

    def test_a_failed_or_cancelled_part_fails(self):
        for result in ("failure", "cancelled"):
            for answer in ("true", "false"):
                with self.subTest(result=result, answer=answer):
                    self.assertFalse(self.passes(LAUNCHER=result, LAUNCHER_ANSWER=answer))
                    self.assertFalse(self.passes(LINT=result, RUST_ANSWER=answer))
                    self.assertFalse(self.passes(LAUNCHER=result, EVENT="merge_group"))

    def test_a_part_skipped_without_a_decision_fails(self):
        # verify-scope failed or was cancelled: what waits on it is skipped, and its outputs are empty.
        for scope in ("failure", "cancelled", "skipped"):
            with self.subTest(scope=scope):
                self.assertFalse(self.passes(SCOPE=scope, LINT="skipped", LAUNCHER="skipped", RUST_ANSWER="",
                                             LAUNCHER_ANSWER=""))
                self.assertFalse(self.passes(SCOPE=scope, LINT="skipped", LAUNCHER="skipped", RUST_ANSWER="false",
                                             LAUNCHER_ANSWER="false"))

    def test_the_merge_queue_shape_passes(self):
        self.assertTrue(self.passes(EVENT="merge_group", LINT="skipped", SERVER="skipped", LAUNCHER="skipped",
                                    RUST_ANSWER="true", LAUNCHER_ANSWER="true"))

    def test_the_other_parts_are_judged_as_before(self):
        for name in ("TREE", "SERVER", "TEST", "MODPOINTS"):
            with self.subTest(part=name):
                self.assertTrue(self.passes(**{name: "skipped"}))
                self.assertFalse(self.passes(**{name: "failure"}))
                self.assertFalse(self.passes(**{name: "cancelled"}))

    def test_every_part_ends_with_a_bounded_load_probe_that_cannot_fail_it(self):
        # Not verify (mod points) yet: another change is reworking that job.
        for part in ("verify-tree", "verify-lint", "verify-server", "verify-launcher", "verify-test"):
            with self.subTest(part=part):
                probe = self.jobs[part]["steps"][-1]
                self.assertEqual(probe["name"], "Load probe")
                self.assertEqual(probe["if"], "always()")
                self.assertIs(probe["continue-on-error"], True)
                self.assertTrue(probe["run"].startswith("timeout -k 5 10 tools/ci/load_probe.sh || echo "), probe["run"])

    def test_the_load_probe_prints_one_line_and_succeeds(self):
        if not BASH:
            self.skipTest("no bash")
        result = subprocess.run([BASH, str(ROOT / "tools/ci/load_probe.sh")], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(result.stdout.splitlines()), 1, result.stdout)
        self.assertTrue(result.stdout.startswith("load probe: "), result.stdout)


class InstallRustTests(unittest.TestCase):
    """tools/ci/install_rust.sh with stand-ins for rustup, sudo and apt-get: apt beside rustup, always reaped, bounded,
    and either one's failure fails the step."""

    STAND_INS = {
        "rustup": 'echo "rustup $*"\nexit "${FAKE_RUSTUP:-0}"\n',
        "sudo": 'echo "sudo $*"\nexec "$@"\n',
        "apt-get": 'echo "apt-get $*"\nsleep "${FAKE_APT_SLEEP:-0}"\nexit "${FAKE_APT:-0}"\n',
    }

    def install(self, *packages, **env):
        if not BASH:
            self.skipTest("no bash")
        with tempfile.TemporaryDirectory() as scratch:
            for name, body in self.STAND_INS.items():
                path = Path(scratch) / name
                path.write_text("#!/usr/bin/env bash\n" + body, newline="\n")
                path.chmod(0o755)
            env = {**os.environ, "HOSTED": "true", **env, "PATH": scratch + os.pathsep + os.environ["PATH"]}
            return subprocess.run([BASH, str(ROOT / "tools/ci/install_rust.sh"), *packages], env=env,
                                  capture_output=True, text=True, timeout=60)

    def test_both_succeed(self):
        result = self.install("libudev-dev", "mold")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("rustup toolchain install stable", result.stdout)
        install = "apt-get install -y --no-install-recommends -o Dpkg::Use-Pty=false libudev-dev mold"
        self.assertIn(install, result.stdout)
        self.assertIn("::group::apt-get (exit 0)", result.stdout)
        # The bound runs as root, so it can stop apt-get and the dpkg it starts, not only sudo.
        self.assertIn("sudo timeout -k 30 300 apt-get update", result.stdout)
        self.assertIn(f"sudo timeout -k 30 300 {install}", result.stdout)

    def test_a_rustup_failure_still_reaps_apt_and_shows_it(self):
        result = self.install("libudev-dev", FAKE_RUSTUP="3", FAKE_APT_SLEEP="1")
        self.assertEqual(result.returncode, 3, result.stdout + result.stderr)
        self.assertIn("::group::apt-get (exit 0)", result.stdout)
        self.assertIn("apt-get install -y --no-install-recommends -o Dpkg::Use-Pty=false libudev-dev", result.stdout)

    def test_an_apt_failure_fails_the_step(self):
        result = self.install("libudev-dev", FAKE_APT="5")
        self.assertEqual(result.returncode, 5, result.stdout + result.stderr)
        self.assertIn("::group::apt-get (exit 5)", result.stdout)

    def test_a_stalled_apt_is_stopped(self):
        result = self.install("libudev-dev", FAKE_APT_SLEEP="40", APT_TIMEOUT="1")
        self.assertEqual(result.returncode, 124, result.stdout + result.stderr)
        self.assertIn("::group::apt-get (exit 124)", result.stdout)

    def test_no_apt_off_blacksmith(self):
        result = self.install("libudev-dev", HOSTED="false")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertNotIn("apt-get", result.stdout)


class TryRunTests(unittest.TestCase):
    """A maintainer's hand-started `checks` run of a branch (docs/ci-runners.md#try-runs) saves no sticky disk, no
    Rust cache, and holds no secret: only main's pushes and the merge queue write what pull requests start from."""

    def test_only_mains_pushes_and_the_merge_queue_save_a_build_disk(self):
        text = (WORKFLOWS / "ci.yml").read_text()
        # Every disk's `commit:` is this variable (tests/test_ci_source_mtimes.py).
        self.assertIn("  SAVE_BUILD_DISK: ${{ ((github.event_name == 'push' && github.ref == 'refs/heads/main') || "
                      "github.event_name == 'merge_group') && 'true' || 'false' }}\n", text)
        self.assertNotIn("secrets.", text)

    def test_mod_point_snapshots_are_kept_only_on_mains_disk(self):
        # A pull request compares with a stored snapshot of its base, so none of its jobs may store one: only the runs
        # that save the disk, and only on Blacksmith, whose disk a pull request mounts but cannot save (our checks
        # runners keep what any job leaves in the build folder).
        steps = dict(jobs((WORKFLOWS / "ci.yml").read_text()))["verify-mod-points"].split("\n      - ")
        store = [step for step in steps if "--store-snapshot" in step]
        read = [step for step in steps if "--snapshots" in step]
        self.assertEqual((len(store), len(read)), (1, 1))
        self.assertIn("if: github.event_name != 'pull_request'\n", store[0])
        self.assertIn("STORE: ${{ env.SAVE_BUILD_DISK == 'true' && startsWith(runner.name, 'blacksmith-') }}\n",
                      store[0])
        self.assertIn('if [ "$STORE" = true ]; then\n', store[0])
        self.assertIn("HOSTED: ${{ startsWith(runner.name, 'blacksmith-') }}\n", read[0])
        self.assertIn('if [ "$HOSTED" = true ]; then\n', read[0])

    def test_the_windows_build_runs_by_hand_and_saves_no_cache(self):
        block = dict(jobs((WORKFLOWS / "ci.yml").read_text()))["windows-x64"]
        self.assertIn("    if: github.event_name == 'workflow_dispatch' && inputs.windows\n", block)
        # Nothing a branch built may reach a cache that main's builds restore.
        self.assertNotIn("rust-cache@", block)
        self.assertNotIn("actions/cache", block)
        self.assertIn("persist-credentials: false", block)
        self.assertIn("retention-days: 3", block)
        # Not the workflow's actions: read.
        self.assertIn("    permissions:\n      contents: read\n", block)


if __name__ == "__main__":
    unittest.main()
