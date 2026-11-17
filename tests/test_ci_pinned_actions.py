"""Every action from outside the repository is pinned to a commit (docs/security.md).

A tag can be moved to other code; a commit can't. Dependabot (.github/dependabot.yml) updates the pins.
"""
import re
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools/ci/vendor/pyyaml-6.0.3.zip"))
import yaml

PINNED = re.compile(r"[^/@\s]+/[^/@\s]+(/[^@\s]+)?@[0-9a-f]{40}")


def uses(document):
    """Every `uses` value of a workflow (jobs and their steps) or an action (runs.steps), parsed, not by line."""
    if not isinstance(document, dict):
        return []
    steps = []
    jobs = document.get("jobs")
    if isinstance(jobs, dict):
        for job in jobs.values():
            if isinstance(job, dict):
                steps.append(job)
                steps.extend(job.get("steps") or [])
    runs = document.get("runs")
    if isinstance(runs, dict):
        steps.extend(runs.get("steps") or [])
    return [step["uses"] for step in steps if isinstance(step, dict) and "uses" in step]


def unpinned(document):
    return [ref for ref in uses(document)
            if not (str(ref).startswith(("./", "docker://")) or PINNED.fullmatch(str(ref)))]


def files():
    github = ROOT / ".github"
    yield from sorted((github / "workflows").glob("*.y*ml"))
    yield from sorted(p for p in github.rglob("action.y*ml"))


class PinnedActionsTests(unittest.TestCase):
    def test_remote_actions_are_pinned_to_a_commit(self):
        found, bad = 0, []
        for path in files():
            document = yaml.safe_load(path.read_text(encoding="utf-8"))
            found += len(uses(document))
            bad += [f"{path.relative_to(ROOT).as_posix()}: {ref}" for ref in unpinned(document)]
        self.assertTrue(found, "no uses found: the collector no longer reads the workflows")
        self.assertEqual(bad, [], "pin these to a full commit SHA, with the version in a comment")

    def test_collector_reads_every_yaml_form(self):
        sha = "0f" * 20
        for source, expected in [
            ('jobs: {a: {steps: [{"uses": actions/checkout@v7}]}}', ["actions/checkout@v7"]),
            ("jobs:\n  a:\n    steps:\n      - &checkout {uses: actions/checkout@v7}\n      - *checkout\n",
             ["actions/checkout@v7"] * 2),
            ("jobs: {a: {uses: o/r/.github/workflows/x.yml@main}}", ["o/r/.github/workflows/x.yml@main"]),
            ("runs: {steps: [{uses: a/b@d1527c8}]}", ["a/b@d1527c8"]),
            (f"jobs:\n  a:\n    steps:\n      - uses: &pin a/b@{sha}\n      - uses: *pin\n", []),
            ("jobs: {a: {uses: ./.github/workflows/x.yml, steps: [{uses: ./tools/a}, {uses: 'docker://x:1'}]}}",
             []),
            (f"jobs: {{a: {{steps: [{{uses: a/b@{sha.upper()}}}]}}}}", [f"a/b@{sha.upper()}"]),
        ]:
            with self.subTest(source=source):
                self.assertEqual(unpinned(yaml.safe_load(source)), expected)


if __name__ == "__main__":
    unittest.main()
