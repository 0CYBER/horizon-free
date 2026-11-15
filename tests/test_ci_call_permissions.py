"""A called workflow may not ask for a permission its caller job does not grant.

GitHub checks this when it expands the call, before any job runs, whatever a nested job's `if:` says, so a
mismatch fails the whole run at startup (dry runs included) and cannot be seen before it reaches main."""
import unittest

from tests.test_ci_concurrency import ROOT, yaml

WORKFLOWS = ROOT / ".github/workflows"
LEVELS = {"none": 0, "read": 1, "write": 2}
SCOPES = ("actions", "attestations", "checks", "contents", "deployments", "discussions", "id-token", "issues",
          "models", "packages", "pages", "pull-requests", "repository-projects", "security-events", "statuses")


def permissions(value):
    """A permissions block as {scope: level}; `read-all`/`write-all` expand to every scope."""
    if value in ("read-all", "write-all"):
        return {scope: value.split("-")[0] for scope in SCOPES}
    return dict(value or {})


def granted(workflow, job):
    """What a caller job grants: its own block, else its workflow's (an absent block is every default, unknown
    here, so it grants nothing this test can rely on)."""
    return permissions(job.get("permissions", workflow.get("permissions", {})))


def bounded(path, grant, via, seen=()):
    """Problems where a job of the called workflow `path` asks for more than `grant`, the effective permissions of
    the job that calls it. A job that is itself a call is bounded first, then bounds its own called workflow with
    what it declares (or, declaring nothing, with what it was given), at every level."""
    workflow = yaml.safe_load(path.read_text())
    top = workflow.get("permissions")
    problems = []
    for name, job in workflow["jobs"].items():
        where = f"{via} -> {path.name}:{name}"
        declared = job.get("permissions", top)
        asked = permissions(declared) if declared is not None else dict(grant)
        for scope, level in asked.items():
            if LEVELS[level] > LEVELS[grant.get(scope, "none")]:
                problems.append(f"{where} asks for {scope}: {level}, but is only granted {scope}: "
                                f"{grant.get(scope, 'none')}")
        if "uses" in job:
            nested = WORKFLOWS / job["uses"].rsplit("/", 1)[1]
            if nested not in seen:
                problems += bounded(nested, asked, where, seen + (path,))
    return problems


def mismatches(caller_path):
    workflow = yaml.safe_load(caller_path.read_text())
    problems = []
    for name, job in workflow["jobs"].items():
        if str(job.get("uses", "")).startswith("./.github/workflows/"):
            problems += bounded(WORKFLOWS / job["uses"].rsplit("/", 1)[1], granted(workflow, job),
                                f"{caller_path.name}:{name}")
    return problems


class CallPermissionTests(unittest.TestCase):
    def test_every_call_grants_what_its_workflow_asks_for(self):
        callers = [path for path in sorted(WORKFLOWS.glob("*.yml")) if "uses: ./.github/workflows/" in path.read_text()]
        self.assertIn(WORKFLOWS / "daily.yml", callers)
        for path in callers:
            with self.subTest(workflow=path.name):
                self.assertEqual(mismatches(path), [])

    def test_a_write_asked_by_a_never_running_job_is_still_a_mismatch(self):
        import tempfile
        from pathlib import Path
        with tempfile.TemporaryDirectory() as folder:
            called = Path(folder) / "called.yml"
            called.write_text("on: workflow_call\npermissions:\n  contents: read\njobs:\n"
                              "  publish:\n    if: false\n    permissions:\n      contents: write\n"
                              "    runs-on: ubuntu-latest\n    steps: [{run: 'true'}]\n")
            caller = Path(folder) / "caller.yml"
            caller.write_text("on: workflow_dispatch\npermissions:\n  contents: read\njobs:\n"
                              "  leg:\n    uses: ./.github/workflows/called.yml\n")
            global WORKFLOWS
            saved, WORKFLOWS = WORKFLOWS, Path(folder)
            try:
                self.assertEqual(mismatches(caller), ["caller.yml:leg -> called.yml:publish asks for contents: write, "
                                                      "but is only granted contents: read"])
            finally:
                WORKFLOWS = saved

    def test_an_elevation_three_levels_deep_is_caught(self):
        import tempfile
        from pathlib import Path
        job = "    runs-on: ubuntu-latest\n    steps: [{run: 'true'}]\n"
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            # caller grants actions: write; the middle call job narrows it to read; the leaf asks for write.
            (root / "caller.yml").write_text("on: workflow_dispatch\njobs:\n  leg:\n    permissions:\n"
                                             "      actions: write\n    uses: ./.github/workflows/middle.yml\n")
            (root / "middle.yml").write_text("on: workflow_call\njobs:\n  inner:\n    permissions:\n"
                                             "      actions: read\n    uses: ./.github/workflows/leaf.yml\n")
            (root / "leaf.yml").write_text("on: workflow_call\njobs:\n  dispatch:\n    permissions:\n"
                                           "      actions: write\n" + job)
            global WORKFLOWS
            saved, WORKFLOWS = WORKFLOWS, root
            try:
                self.assertEqual(mismatches(root / "caller.yml"), [
                    "caller.yml:leg -> middle.yml:inner -> leaf.yml:dispatch asks for actions: write, "
                    "but is only granted actions: read"])
                (root / "middle.yml").write_text("on: workflow_call\njobs:\n  inner:\n"
                                                 "    uses: ./.github/workflows/leaf.yml\n")
                self.assertEqual(mismatches(root / "caller.yml"), [], "a call job declaring nothing passes on its grant")
            finally:
                WORKFLOWS = saved


if __name__ == "__main__":
    unittest.main()
