"""Runs that build a commit other than main's own restore caches and do not save them.

GitHub scopes Actions caches by the ref a run was started on: a run started on main that checks out and builds another
commit would write entries main's later runs (releases, the Nightly, the launcher, the SDK) restore. So every job that
can check out code other than its run's own commit either saves no cache or saves only under a condition that holds for
main's own commit. A job "can check out other code" when a checkout's `ref:` is anything but the run's own commit, or
a step fetches, switches or adds a worktree with git. Each such job with a cache step that can save is classified
below; a new one fails `test_every_job_that_builds_other_code_and_can_save_is_classified` until it is.

This reads the workflow files only: code a job builds could still call the cache service itself, which no workflow
setting stops.
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
sys.path.insert(0, str(ROOT / "tools/ci/vendor/pyyaml-6.0.3.zip"))
import yaml  # noqa: E402
WORKFLOWS = ROOT / ".github/workflows"
# On Windows the bash on PATH is Git's; a bare "bash" there would be WSL's (tests/test_pr_ready.py).
BASH = shutil.which("bash")
# Checkout refs that are the run's own commit.
OWN_COMMIT = {None, "${{ github.sha }}"}
# `git -C dir fetch`, `git -c k=v checkout`: options that take a separate value are skipped with it.
OTHER_CODE = re.compile(r"\bgit\s+(?:(?:-[Cc]\s+\S+|-\S+)\s+)*"
                        r"(?:fetch|clone|checkout|switch|worktree|reset|pull|merge|cherry-pick|am)\b"
                        r"|\bgh\s+pr\s+checkout\b")


def workflows():
    for path in sorted([*WORKFLOWS.glob("*.yml"), *WORKFLOWS.glob("*.yaml")]):
        yield path.name, yaml.safe_load(path.read_text())


def action(step):
    return step.get("uses", "").split("@")[0]


def can_save(step):
    """Whether a step can write a cache entry: False, True, or the condition it saves under."""
    name, given = action(step), step.get("with", {}) or {}
    if name == "Swatinem/rust-cache":
        return given.get("save-if", True)
    if name in ("actions/cache", "actions/cache/save"):
        return step.get("if", True)
    if name == "useblacksmith/stickydisk":
        return given.get("commit", True)
    if name == "actions/cache/restore":
        return False
    # Any other cache action (a compiler cache's, say) is taken to save.
    if "cache" in name.lower():
        return True
    # setup-python, setup-java, setup-node and the like: a `cache` input saves at the end of the job.
    return any("cache" in key for key in given) or False


def other_code(job):
    for step in job.get("steps", []):
        given = step.get("with", {}) or {}
        if action(step) == "actions/checkout" and (given.get("ref") not in OWN_COMMIT or "repository" in given):
            return True
        if OTHER_CODE.search(str(step.get("run", ""))):
            return True
    return False


def saving_jobs_with_other_code():
    for name, workflow in workflows():
        for job_name, job in workflow["jobs"].items():
            saves = [step for step in job.get("steps", []) if can_save(step) is not False]
            if saves and other_code(job):
                yield f"{name} {job_name}", job, saves


def step(job, step_id):
    return next(s for s in job["steps"] if s.get("id") == step_id)


class CacheSaveTests(unittest.TestCase):
    # How each job that can build other code keeps its saves to main's own commit.
    CLASSIFIED = {
        # Its `ref` is the caller's: saves only when the caller says `trusted` (see the callers' test below).
        "game-build.yml build",
        # The commit is the run's own (a push, a dry run) or main's newest one with green checks
        # (release_train.py green_main_head, from main's push runs).
        "launcher.yml build",
        # The head is the plan's checkout of the run's own commit; saves only on main: pinned by
        # tests/test_ci_runners.py HostedParityTests.test_only_mains_runs_save.
        "parity-hosted.yml run",
        # Builds any `base`: saves only when that base is main's own commit of the run.
        "pack-bench.yml bench",
    }

    def test_every_job_that_builds_other_code_and_can_save_is_classified(self):
        found = {where for where, _, _ in saving_jobs_with_other_code()}
        self.assertEqual(found, self.CLASSIFIED)

    def test_the_game_build_saves_only_for_a_trusted_caller_and_no_caller_trusts_another_commit(self):
        job = yaml.safe_load((WORKFLOWS / "game-build.yml").read_text())["jobs"]["build"]
        caches = [s for s in job["steps"] if can_save(s) is not False]
        self.assertEqual([can_save(s) for s in caches], ["${{ inputs.trusted }}"])
        callers = 0
        for name, workflow in workflows():
            for job_name, caller in workflow["jobs"].items():
                if caller.get("uses") == "./.github/workflows/game-build.yml":
                    callers += 1
                    given = caller.get("with", {})
                    # Only a build of the run's own commit may be trusted; the Nightly's set never is.
                    if given.get("trusted", False) is not False:
                        self.assertEqual(given["ref"], "${{ github.sha }}", f"{name} {job_name}")
        self.assertGreater(callers, 0)
        nightly = yaml.safe_load((WORKFLOWS / "nightly-build.yml").read_text())["jobs"]["build"]
        self.assertIs(nightly["with"]["trusted"], False)

    def test_the_launcher_builds_the_runs_own_commit_or_mains(self):
        decide = yaml.safe_load((WORKFLOWS / "launcher.yml").read_text())["jobs"]["decide"]
        own = "${{ (github.event_name == 'push' || steps.mode.outputs.dry_run == 'true') && github.sha || '' }}"
        self.assertEqual(step(decide, "decide")["env"]["HEAD_SHA"], own)

class PackBenchCacheTests(unittest.TestCase):
    """A dispatch from main may compare with any base, a pull request's head too; only a run whose base is main's own
    commit of the run saves the Rust cache."""

    URL = "https://github.com/horizon-rust/horizon.git"

    def setUp(self):
        self.job = yaml.safe_load((WORKFLOWS / "pack-bench.yml").read_text())["jobs"]["bench"]
        self.gate = step(self.job, "gate")

    def test_the_cache_saves_only_on_the_gates_answer_decided_before_it(self):
        steps = self.job["steps"]
        saves = [s for s in steps if can_save(s) is not False]
        self.assertEqual([can_save(s) for s in saves], ["${{ steps.gate.outputs.saves == 'true' }}"])
        self.assertLess(steps.index(self.gate), steps.index(saves[0]))
        # Nothing fetched, checked out or built before the gate.
        for earlier in steps[:steps.index(self.gate)]:
            self.assertNotIn("cargo", str(earlier.get("run", "")))
            self.assertFalse(OTHER_CODE.search(str(earlier.get("run", ""))), earlier.get("name"))

    def test_the_base_built_is_the_one_the_gate_judged(self):
        bench = next(s for s in self.job["steps"] if s.get("name") == "Bench the base, with the branch's bench")
        self.assertEqual(bench["env"]["BASE_SHA"], "${{ steps.gate.outputs.base }}")
        # Detached at the SHA: a local branch whose name looks like a SHA is never what gets checked out.
        self.assertIn('git worktree add --detach ../base "$BASE_SHA"', bench["run"])
        self.assertNotIn("git fetch", bench["run"])
        self.assertNotIn("FETCH_HEAD", bench["run"])

    def run_gate(self, repo, ref, sha, base, check=True):
        out = Path(repo) / "out"
        out.write_text("")
        script = self.gate["run"].replace(self.URL, repo)
        self.assertNotEqual(script, self.gate["run"])
        env = dict(os.environ, REF=ref, BASE=base, GITHUB_SHA=sha, GITHUB_OUTPUT=str(out))
        done = subprocess.run([BASH, "-c", script], cwd=Path(repo) / "clone", env=env, check=check,
                              capture_output=True)
        if not check:
            return done.returncode, out.read_text()
        return dict(line.split("=", 1) for line in out.read_text().splitlines())

    @unittest.skipUnless(BASH, "needs bash")
    def test_only_mains_own_commit_saves(self):
        with tempfile.TemporaryDirectory() as tmp:
            git = lambda *args, cwd=tmp: subprocess.run(
                ["git", "-c", "user.name=t", "-c", "user.email=t@t", *args], cwd=cwd, check=True,
                capture_output=True, text=True).stdout.strip()
            git("init", "-q", "-b", "main")
            git("commit", "-q", "--allow-empty", "-m", "main")
            main = git("rev-parse", "HEAD")
            git("checkout", "-q", "-b", "other")
            git("commit", "-q", "--allow-empty", "-m", "other")
            other = git("rev-parse", "HEAD")
            git("update-ref", "refs/pull/1/head", other)
            git("checkout", "-q", "main")
            git("clone", "-q", tmp, str(Path(tmp) / "clone"))
            repo = Path(tmp).as_posix()
            self.assertEqual(self.run_gate(repo, "refs/heads/main", main, "main"), {"base": main, "saves": "true"})
            self.assertEqual(self.run_gate(repo, "refs/heads/main", main, main), {"base": main, "saves": "true"})
            for base in ("other", "refs/pull/1/head", other):
                self.assertEqual(self.run_gate(repo, "refs/heads/main", main, base),
                                 {"base": other, "saves": "false"}, base)
            # Main moved on after the run started: the newer main is not this run's commit.
            self.assertEqual(self.run_gate(repo, "refs/heads/main", other, "main")["saves"], "false")
            # Off main nothing saves, its own commit as base included.
            self.assertEqual(self.run_gate(repo, "refs/heads/other", other, "other")["saves"], "false")
            self.assertEqual(self.run_gate(repo, "refs/heads/Main", main, "main")["saves"], "false")
            # Anything but one source ref or SHA is refused before a fetch: a destination refspec, a second ref (the
            # run's own SHA first, then another commit written to a branch named like it), an option, a forced
            # update, whitespace, an empty base.
            for base in (f"other:refs/heads/{main}", f"{main} other:refs/heads/{main}", f"{main} other",
                         "+other", "-v", "--upload-pack=x", " main", "main\n", ""):
                code, out = self.run_gate(repo, "refs/heads/main", main, base, check=False)
                self.assertNotEqual(code, 0, repr(base))
                self.assertEqual(out, "", repr(base))
            self.assertEqual(git("for-each-ref", "--format=%(refname)", "refs/heads", cwd=Path(tmp) / "clone"),
                             "refs/heads/main")


if __name__ == "__main__":
    unittest.main()
