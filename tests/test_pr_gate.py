"""A contributor's pull request gets only the cheap checks; rollups, syncs, main and dispatches get the heavy ones."""

import importlib.util
import itertools
import json
import os
import re
import subprocess
import time
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("pr_gate", ROOT / "tools/ci/pr_gate.py")
pr_gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(pr_gate)

SHA = "a" * 40
SAME_REPO = "github.event_name != 'pull_request' || github.event.pull_request.head.repo.full_name == github.repository"


def never_read(_):
    raise AssertionError("only a dispatch with pr reads a pull request")


class GateTests(unittest.TestCase):
    def test_pull_request_gate(self):
        cases = [
            (True, ["rollup"], "work", "true"),
            (True, [], "fork-review-rollup-42", "true"),
            (True, ["sync"], "work", "true"),
            (True, [], "sync-28-35", "true"),
            (True, [], "rollup-76", "true"),
            (True, [], "rollup-77", "true"),
            (True, [], "rollupx", "false"),
            (True, [], "cfix-hooks-rollup-76", "false"),
            (False, [], "rollup-76", "false"),
            (True, ["bug"], "ci-gate", "false"),
            (False, ["rollup", "sync"], "fork-review-rollup-42", "false"),
        ]
        for same, labels, branch, expected in cases:
            with self.subTest(same=same, labels=labels, branch=branch):
                result = pr_gate.outputs("pull_request", same, "", never_read,
                                         labels=labels, head_ref=branch)
                self.assertEqual(result, {"heavy": expected, "pr_sha": "", "trusted": ""})
                # An empty trusted output preserves the workflow's repository-based trust fallback.
        for event in ("push", "schedule", "workflow_dispatch"):
            self.assertEqual(pr_gate.outputs(event, False, "", never_read)["heavy"], "true")

    def test_a_dispatch_with_pr_runs_that_head_and_keeps_a_fork_off_our_runners(self):
        fork = pr_gate.outputs("workflow_dispatch", True, "3665", lambda n: {"sha": SHA, "same_repository": False})
        self.assertEqual(fork, {"heavy": "true", "pr_sha": SHA, "trusted": "false"})
        ours = pr_gate.outputs("workflow_dispatch", True, "3672", lambda n: {"sha": SHA, "same_repository": True})
        self.assertEqual(ours, {"heavy": "true", "pr_sha": SHA, "trusted": "true"})

    def test_pr_must_be_a_number(self):
        for bad in ("0", "12a", "1;id", "refs/pull/1/head", "-1"):
            with self.subTest(pr=bad), self.assertRaises(SystemExit):
                pr_gate.outputs("workflow_dispatch", True, bad, never_read)


class WorkflowTests(unittest.TestCase):
    workflow = (ROOT / ".github/workflows/dyna-ci.yml").read_text()

    def job(self, name):
        match = re.search(rf"^  {re.escape(name)}:\n(.*?)(?=^  [a-z0-9_-]+:\n|\Z)", self.workflow, re.M | re.S)
        self.assertIsNotNone(match, name)
        return match[1]

    def test_heavy_jobs_wait_for_the_gate_and_fail_closed(self):
        for name in ("lint", "verify"):
            with self.subTest(job=name):
                self.assertIn("(needs.changes.result != 'success' || needs.changes.outputs.heavy == 'true')",
                              self.job(name))
        self.assertIn("needs.changes.outputs.heavy == 'true'", self.job("launcher"))
        for name in ("vanilla-parity",):
            self.assertIn("!inputs.pr", self.job(name), name)

    def test_cheap_jobs_run_without_the_heavy_gate(self):
        self.assertNotIn("outputs.heavy", self.job("verify-static"))
        self.assertIn("needs.changes.outputs.heavy != 'true'", self.job("format"))
        self.assertNotRegex(self.job("format"), r"cargo (clippy|build|test|check)")

    def test_a_dispatched_head_is_checked_out_with_a_read_only_token_and_reported_apart(self):
        for name in ("lint", "verify", "verify-static", "launcher"):
            with self.subTest(job=name):
                self.assertIn("ref: ${{ needs.changes.outputs.pr_sha }}", self.job(name))
                self.assertNotIn("statuses: write", self.job(name))
        status = self.job("pr-status")
        self.assertIn("statuses: write", status)
        self.assertNotIn("actions/checkout", status)
        self.assertIn("permissions:\n  contents: read\n", self.workflow)

    def test_no_fork_code_reaches_our_runners(self):
        # The one trust decision: the gate's answer for a dispatched pull request, else the event's own repository.
        self.assertIn(f"trusted: ${{{{ steps.gate.outputs.trusted || (({SAME_REPO}) && 'true' || 'false') }}}}",
                      self.job("changes"))
        self.assertNotIn("github.event.pull_request.head.repo.full_name == github.repository) &&", self.job("lint"))
        for name in ("lint", "verify", "verify-static", "launcher", "boat-watch"):
            body = self.job(name)
            if "head.repo.full_name" in body:
                self.fail(f"{name} decides trust itself instead of needs.changes.outputs.trusted")

    def runs_on(self, name, outputs, self_hosted_ci):
        """Evaluates a job's runs-on expression with these changes outputs, the way Actions would."""
        expression = re.search(r"^    runs-on: \$\{\{ (.*) \}\}$", self.job(name), re.M)[1]
        names = {"needs.changes.outputs." + key: value for key, value in outputs.items()}
        names |= {"vars.SELF_HOSTED_CI": self_hosted_ci, "inputs.github-fallback": False}
        python = re.sub(r"fromJSON\('([^']*)'\)", lambda m: repr(json.loads(m[1])), expression)
        python = re.sub(r"[a-z]+(?:\.[A-Za-z_-]+)+", lambda m: repr(names[m[0]]), python)
        python = python.replace("&&", " and ").replace("||", " or ").replace("!=", " != ")
        python = re.sub(r"!(?!=)", " not ", python)
        return eval(python, {})

    def test_an_untrusted_head_never_gets_our_runners(self):
        # A fork's dispatched pr: trusted is false and the runner script answers self_hosted=false.
        for name in ("lint", "verify", "verify-static", "launcher"):
            for rust, forced, label, pool in itertools.product(("true", "false"), ("true", "false"),
                                                               ("", "boat"), ("all", "")):
                outputs = {"rust_required": rust, "forced_github": forced, "hosted_label": label,
                           "self_hosted": "false", "trusted": "false"}
                with self.subTest(job=name, outputs=outputs, pool=pool):
                    runner = self.runs_on(name, outputs, pool)
                    # verify-static, one of a contributor's two checks, never waits on a paid pool either.
                    self.assertIn(runner, ("ubuntu-latest",) if name == "verify-static" else ("ubuntu-latest", "boat"))
        # The control: the same expressions do reach our runners for a trusted head.
        trusted = {"rust_required": "true", "forced_github": "false", "hosted_label": "",
                   "self_hosted": "true", "trusted": "true"}
        self.assertEqual(self.runs_on("verify", trusted, "all"), "dyna-large")

    def test_the_runner_script_hears_the_gates_trust(self):
        self.assertIn("SAME_REPO: ${{ steps.gate.outputs.trusted || (", self.job("changes"))
        lease = str(int(time.time()) + 3600)
        for same_repo, expected in (("false", "self_hosted=false"), ("true", "self_hosted=true")):
            env = {"PATH": os.environ["PATH"], "ENABLED": "true", "SAME_REPO": same_repo,
                   "EVENT": "workflow_dispatch", "LEASE": lease}
            result = subprocess.run(["bash", ROOT / "tools/ci/dyna_ci_runner.sh"], env=env, capture_output=True,
                                    text=True, check=True)
            self.assertEqual(result.stdout.strip(), expected, same_repo)

    def test_a_contributors_cheap_checks_run_on_githubs_runners(self):
        # format and verify-static are all a fork pull request gets; a paid pool's balance must not fail them.
        self.assertIn("runs-on: ubuntu-latest\n", self.job("format"))
        static = self.job("verify-static")
        self.assertIn("needs.changes.outputs.trusted != 'true' && 'ubuntu-latest' ||", static)
        self.assertIn("RUNNER_KIND: ${{ needs.changes.outputs.trusted != 'true' && 'github' ||", static)

    def test_windows_jobs_share_the_rollup_and_sync_gate(self):
        import yaml

        workflow = yaml.safe_load((ROOT / ".github/workflows/launcher-windows-check.yml").read_text())
        jobs = workflow["jobs"]
        gate = jobs["gate"]
        step = next(step for step in gate["steps"] if step.get("id") == "gate")
        self.assertEqual(step["run"], 'python3 tools/ci/pr_gate.py >> "$GITHUB_OUTPUT"')
        self.assertEqual(step["env"], {
            "EVENT": "${{ github.event_name }}",
            "HEAD_REPO": "${{ github.event.pull_request.head.repo.full_name }}",
            "PR_LABELS": "${{ toJSON(github.event.pull_request.labels.*.name) }}",
            "HEAD_REF": "${{ github.event.pull_request.head.ref }}",
        })
        self.assertEqual(gate["outputs"]["heavy"], "${{ steps.gate.outputs.heavy }}")
        self.assertEqual(jobs["test"]["needs"], "gate")
        self.assertEqual(jobs["test"]["if"], "inputs.commit == '' && needs.gate.outputs.heavy == 'true'")
        # It starts child processes: the daily run calls it, and a hand dispatch runs it; never pushes or PRs.
        for name in ("runtime-leases", "launcher-processes"):
            self.assertEqual(jobs[name]["if"], "inputs.commit != '' || github.event_name == 'workflow_dispatch'")

    def test_each_launcher_test_target_runs_once_and_spawning_ones_run_daily(self):
        import yaml

        jobs = yaml.safe_load((ROOT / ".github/workflows/launcher-windows-check.yml").read_text())["jobs"]
        def targets(job):
            return {arg for step in jobs[job]["steps"] for arg in re.findall(r"--test (\w+)", step.get("run", ""))}
        spawning = {path.stem for path in (ROOT / "launcher/tests").glob("*.rs")
                    if "CARGO_BIN_EXE" in path.read_text() or "Command::new" in path.read_text()}
        every = {path.stem for path in (ROOT / "launcher/tests").glob("*.rs")}
        self.assertEqual(targets("launcher-processes"), spawning)
        self.assertEqual(targets("test"), every - spawning)
        # The product reserves the stack the debug launcher needs (#4052): no release-only workaround.
        self.assertNotIn("--release", next(s["run"] for s in jobs["launcher-processes"]["steps"] if "--test" in s.get("run", "")))

    def test_other_pull_request_workflows_skip_what_compiles_for_forks(self):
        for workflow, jobs in (("cargo-horizon-mod.yml", ("build",)), ("modding-book.yml", ("book",))):
            text = (ROOT / ".github/workflows" / workflow).read_text()
            for name in jobs:
                with self.subTest(workflow=workflow, job=name):
                    body = re.search(rf"^  {name}:\n(.*?)(?=^  [a-z0-9_-]+:\n|\Z)", text, re.M | re.S)[1]
                    self.assertIn(f"if: {SAME_REPO}\n", body)


if __name__ == "__main__":
    unittest.main()
