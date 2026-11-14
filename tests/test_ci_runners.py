"""Which jobs may run on our own runners (docs/ci-runners.md).

- horizon-light (playhorizon's and other small runners, which refuse a fork's pull request): main's own scripts only.
- horizon-checks (isolated runners that take any pull request, forks included): no secret and no token that writes.
- Neither takes a job that holds a secret or whose token can push, publish or start workflows: playhorizon is a
  partner's server, and a checks runner keeps what a fork's job left behind.
"""
import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
WORKFLOWS = ROOT / ".github/workflows"
LIGHT = ("horizon-light", "playhorizon-light", "playhorizon-ci")
CHECKS = ("horizon-checks",)
FORK_GUARD = "github.event.pull_request.head.repo.full_name == github.repository"
# dyna-ci.yml decides trust once, in its changes job (a fork's pull request, dispatched with `pr` or not).
TRUSTED = "needs.changes.outputs.trusted == 'true'"
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
    def test_workflows_do_not_classify_pull_requests_by_fork_flag(self):
        for path in sorted(WORKFLOWS.glob("*.y*ml")):
            with self.subTest(workflow=path.name):
                self.assertNotIn("head.repo.fork", path.read_text())

    def test_every_job_on_our_runners_can_go_back_to_github(self):
        count = 0
        for where, _, _, _, value, pools in routed():
            count += 1
            # dyna-ci.yml (ours) routes through the changes job's self_hosted
            # output and dyna-medium, or SELF_HOSTED_CI=all with the fork guard
            # before the playhorizon-ci labels; the carried upstream files use
            # SELF_HOSTED_CI != 'off'. Both fall back to GitHub's runners.
            if "dyna-ci.yml" in where:
                self.assertIn("needs.changes.outputs.self_hosted == 'true'", value, where)
                self.assertIn("vars.SELF_HOSTED_CI == 'all'", value, where)
                self.assertIn(TRUSTED, value, where)
            else:
                self.assertIn("vars.SELF_HOSTED_CI != 'off'", value, where)
            # Both labels select a GitHub-hosted Ubuntu runner when our pools are disabled.
            self.assertRegex(value, r"\|\| 'ubuntu-(?:latest|24\.04)' \}\}$", where)
            if "checks" in pools:
                # The desktop paused: every checks job back to GitHub, the bots stay.
                self.assertIn("vars.SELF_HOSTED_CI != 'light'", value, where)
        self.assertGreaterEqual(count, 10)

    def test_light_runners_take_main_code_only(self):
        for where, on, _, block, value, pools in routed():
            if "light" not in pools:
                continue
            if "pull_request" in on:
                # The guard lives in the job's `if` upstream; dyna-ci.yml (ours)
                # puts it in `runs-on`, before the self-hosted labels.
                condition = next((l for l in block.splitlines() if l.strip().startswith("if:")), "")
                guarded = any(guard in condition or guard in value for guard in (FORK_GUARD, TRUSTED))
                self.assertTrue(guarded, f"{where}: a fork's pull request")
            if "pull_request_target" in on:
                # main's code only: every checkout is main's.
                self.assertEqual(block.count("uses: actions/checkout"), block.count("ref: main"), where)

    def test_no_secret_or_writing_token_on_our_runners(self):
        for where, _, text, block, _, pools in routed():
            self.assertNotIn("secrets.", block, where)
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

    def test_no_deployment_environment_on_our_runners(self):
        import hashlib
        import sys
        from unittest.mock import patch

        # Reuse the repository's pinned offline parser, including YAML merge semantics.
        archive = ROOT / "tools/ci/vendor/pyyaml-6.0.3.zip"
        self.assertEqual(hashlib.sha256(archive.read_bytes()).hexdigest(),
                         "94ac21b4343e15647b3b5b5bd594aab5b7d3cd8d2ce17db15acb7260d489b6fc")
        with patch.object(sys, "path", [str(archive), *sys.path]):
            import yaml
        for path in sorted(WORKFLOWS.glob("*.y*ml")):
            document = yaml.safe_load(path.read_text())
            for name, job in document["jobs"].items():
                value = job.get("runs-on", "")
                if isinstance(value, dict):
                    value = value.get("labels", [])
                routes = value if isinstance(value, list) else [value]
                if any(route == label or f'"{label}"' in route or f"'{label}'" in route
                       for route in routes for label in LIGHT + CHECKS):
                    self.assertNotIn("environment", job, f"{path.name} {name}")

    def test_deployment_environment_guard_distinguishes_script_variables(self):
        import tempfile
        from unittest.mock import patch

        safe = ("jobs:\n  verify:\n    runs-on: horizon-checks\n    steps:\n"
                "      - run: |\n          with open('output') as environment:\n              pass\n")
        forbidden = [
            safe.replace("    steps:", f"    {key} deployment\n    steps:")
            for key in ("environment:", "\"environment\":", "'environment' :")
        ] + [
            "jobs:\n    verify:\n        runs-on: horizon-checks\n        environment: deployment\n",
            "jobs:\n  verify: {runs-on: horizon-checks, environment: deployment}\n",
            "env: &defaults {environment: deployment}\njobs:\n  verify:\n"
            "    <<: *defaults\n    runs-on: horizon-checks\n",
        ]
        with tempfile.TemporaryDirectory() as directory, patch(f"{__name__}.WORKFLOWS", Path(directory)):
            workflow = Path(directory) / "fixture.yml"
            workflow.write_text(safe)
            self.test_no_deployment_environment_on_our_runners()
            for text in forbidden:
                with self.subTest(workflow=text):
                    workflow.write_text(text)
                    with self.assertRaises(AssertionError):
                        self.test_no_deployment_environment_on_our_runners()

    def test_checks_split_between_both_pools(self):
        ci = dict(jobs((WORKFLOWS / "ci.yml").read_text()))
        value = runs_on(ci["launcher"])
        # A pull request by the last digit of its number unless SELF_HOSTED_CI=all; main's pushes keep GitHub's cache.
        self.assertIn("github.event_name == 'pull_request' && (", value)
        self.assertIn("vars.SELF_HOSTED_CI == 'all'", value)
        self.assertIn("endsWith(format('{0}', github.event.pull_request.number), '0')", value)
        self.assertNotIn(FORK_GUARD, value)
        # verify's parts go where the launcher goes; verify itself only waits for them.
        for part in ("verify-tree", "verify-lint", "verify-test"):
            with self.subTest(part=part):
                self.assertEqual(runs_on(ci[part]), value)
        self.assertIn("needs: [verify-tree, verify-lint, verify-test]", ci["verify"])
        # Never skipped: GitHub counts a skipped required check as passed.
        self.assertIn("if: always()", ci["verify"])

    def test_the_light_runners_refuse_forks_themselves(self):
        script = (ROOT / "tools/ci/checks-runners.sh").read_text()
        self.assertIn("ACTIONS_RUNNER_HOOK_JOB_STARTED=$shared/job-started.sh", script)
        self.assertIn('.pull_request.head.repo.full_name // "unknown"', script)
        self.assertIn('if [ "$head" != "${GITHUB_REPOSITORY:-}" ]', script)


if __name__ == "__main__":
    unittest.main()
