"""Which jobs may run on our own runners (docs/ci-runners.md).

- horizon-light (playhorizon's and other small runners, which refuse a fork's pull request): main's own scripts only.
- horizon-checks (isolated runners; an approved fork's pull request ending in 0 runs there too): no secret and
  no token that writes.
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
            self.assertIn("vars.SELF_HOSTED_CI != 'off'", value, where)
            # Both labels select a GitHub-hosted Ubuntu runner when our pools are disabled.
            self.assertRegex(value, r"\|\| 'ubuntu-(?:latest|24\.04)' \}\}$", where)
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

    def test_checks_split_between_both_pools(self):
        ci = dict(jobs((WORKFLOWS / "ci.yml").read_text()))
        value = runs_on(ci["verify-static"])
        # A pull request by the last digit of its number unless SELF_HOSTED_CI=all; main's pushes keep GitHub's cache.
        self.assertIn("github.event_name == 'pull_request' && ", value)
        self.assertIn("vars.SELF_HOSTED_CI == 'all'", value)
        self.assertIn("endsWith(format('{0}', github.event.pull_request.number), '0')", value)
        # The whole merge queue, unless SELF_HOSTED_CI is light or off.
        self.assertIn("github.event_name == 'merge_group') &&", value)
        self.assertNotIn("head_ref", value)
        # verify's parts all go where the first goes; verify itself only waits for them.
        for part in ("verify-static", "verify-test"):
            with self.subTest(part=part):
                self.assertEqual(runs_on(ci[part]), value)
        self.assertIn("needs: [verify-static, verify-test]", ci["verify"])
        # Never skipped: GitHub counts a skipped required check as passed.
        self.assertIn("if: always()", ci["verify"])

    def test_forks_reach_only_the_checks_runners(self):
        # An approved fork's pull request ending in 0 runs on horizon-checks like this repository's own (GitHub's
        # 20-job pool is not the only place it may wait). Never the light runners: every light job that a pull request
        # can start carries the fork guard (test_light_runners_take_main_code_only), and ci.yml's two runs-on
        # expressions (the other workflows keep their own guards) no longer test the head repository.
        ci = dict(jobs((WORKFLOWS / "ci.yml").read_text()))
        for name in ("verify-static", "verify-test"):
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


if __name__ == "__main__":
    unittest.main()
