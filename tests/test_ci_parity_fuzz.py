"""parity-fuzz.yml handles the vanilla data by parity-hosted.yml's rules: the step that holds a secret is that file's,
line for line, and what it leaves out stays out (no other trigger, no cache read or written, no Cargo build beside the
data, no secret or data in the build job)."""
import os
import re
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
# Use the same offline parser as the repository guard, not a runner package.
sys.path.insert(0, str(ROOT / "tools/ci/vendor/pyyaml-6.0.3.zip"))
import yaml

FUZZ = ROOT / ".github/workflows/parity-fuzz.yml"
HOSTED = ROOT / ".github/workflows/parity-hosted.yml"
SECRET = re.compile(r"secrets\.[A-Z_]+")
GIT_BASH = Path("C:/Program Files/Git/bin/bash.exe")
BASH = str(GIT_BASH) if os.name == "nt" and GIT_BASH.exists() else "bash"


def steps(workflow, job):
    return workflow["jobs"][job]["steps"]


def named(workflow, job, name):
    found = [step for step in steps(workflow, job) if step.get("name") == name]
    assert len(found) == 1, (job, name, len(found))
    return found[0]


class ParityFuzzWorkflowTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.text = FUZZ.read_text(encoding="utf-8")
        cls.fuzz = yaml.safe_load(cls.text)
        cls.hosted = yaml.safe_load(HOSTED.read_text(encoding="utf-8"))

    def test_a_hand_start_is_the_only_trigger_and_the_token_only_reads(self):
        # PyYAML reads the key `on` as True.
        self.assertEqual(list(self.fuzz[True]), ["workflow_dispatch"])
        self.assertEqual(self.fuzz["permissions"], {"contents": "read"})
        for name, job in self.fuzz["jobs"].items():
            self.assertNotIn("permissions", job, name)
        self.assertNotIn("pull_request", self.text)

    def test_the_plan_steps_are_hosted_paritys(self):
        for name in ("Access to the vanilla data", "The commit of the trusted scripts",
                     "The commit to test, on a branch of this repository"):
            self.assertEqual(named(self.fuzz, "plan", name), named(self.hosted, "plan", name), name)

    def test_the_plan_fails_without_the_data_and_every_other_job_needs_it(self):
        plan = steps(self.fuzz, "plan")
        names = [step.get("name") for step in plan]
        gate = names.index("No run without the data")
        self.assertEqual(gate, names.index("Access to the vanilla data") + 1)
        self.assertNotIn("if", plan[gate])
        for data, code in (("true", 0), ("false", 1), ("", 1)):
            result = subprocess.run([BASH, "-c", plan[gate]["run"]], env={**os.environ, "DATA": data},
                                    capture_output=True, text=True)
            self.assertEqual(result.returncode, code, (data, result.stdout, result.stderr))
        for job in ("build", "fuzz", "summary"):
            needs = self.fuzz["jobs"][job]["needs"]
            self.assertIn("plan", [needs] if isinstance(needs, str) else needs)

    def test_the_fetch_step_is_hosted_paritys(self):
        for name in ("Folders for the data, outside the checkout", "Fetch the vanilla data"):
            self.assertEqual(named(self.fuzz, "fuzz", name), named(self.hosted, "run", name), name)

    def test_only_the_fetch_step_holds_a_secret(self):
        holders = {}
        for job in self.fuzz["jobs"]:
            for step in steps(self.fuzz, job):
                found = set(SECRET.findall(yaml.safe_dump(step)))
                if found:
                    holders[(job, step.get("name"))] = found
        key, passphrase = "secrets.VANILLA_DATA_DEPLOY_KEY", "secrets.VANILLA_DATA_PASSPHRASE"
        self.assertEqual(holders, {
            ("plan", "Access to the vanilla data"): {key, passphrase},  # only whether they are set
            ("fuzz", "Fetch the vanilla data"): {key, passphrase},
        })
        # Nowhere outside a step either (a job's or the workflow's env would reach every step).
        self.assertEqual(len(SECRET.findall(self.text)), 4)
        self.assertNotIn("secrets: inherit", self.text)

    def test_before_the_secret_the_runner_has_only_pinned_actions_work(self):
        fuzz = steps(self.fuzz, "fuzz")
        secret = [i for i, step in enumerate(fuzz) if SECRET.search(yaml.safe_dump(step))]
        self.assertEqual(len(secret), 1)
        before = fuzz[:secret[0]]
        # No file of the tested commit, of the build job or of a cache: the JDK's action, and shell that names none.
        self.assertEqual([step["uses"].split("@")[0] for step in before if "uses" in step], ["actions/setup-java"])
        for step in before:
            self.assertNotRegex(step.get("run", ""), r"python3?\b|cargo\b|bash \S|\./", step.get("name"))
        self.assertIn("actions/checkout@", str(fuzz[secret[0] + 1].get("uses", "")))

    def test_no_job_reads_or_writes_a_cache(self):
        # A restore before the secret step would take what another job of the run (the build job's code) wrote in
        # the branch's cache scope: actions/cache unpacks an entry with absolute paths.
        self.assertNotIn("actions/cache", self.text)
        # The plan's copied step only asks whether the sealing script exists; no job fetches or runs it.
        for job in ("build", "fuzz", "summary"):
            self.assertNotIn("sealed-cache", yaml.safe_dump(self.fuzz["jobs"][job]), job)
        self.assertNotIn("sccache", self.text.replace("HORIZON_NO_SCCACHE", ""))
        self.assertEqual(self.fuzz["cache-mode"], "none")
        for name, job in self.fuzz["jobs"].items():
            self.assertEqual(job["cache-mode"], "none", name)

    def test_cargo_builds_only_in_the_job_without_secret_or_data(self):
        build = yaml.safe_dump(self.fuzz["jobs"]["build"])
        for word in ("secrets.", "VANILLA", "download-artifact", "GH_TOKEN", "github.token"):
            self.assertNotIn(word, build)
        self.assertEqual(self.fuzz["jobs"]["build"]["needs"], "plan")
        made = named(self.fuzz, "build", "The game in the fast profile, and the preparer")["run"]
        self.assertIn("cargo build --release --locked -p horizon-prepare", made)
        for job in ("plan", "fuzz", "summary"):
            for step in steps(self.fuzz, job):
                self.assertNotRegex(step.get("run", ""), r"\bcargo\b", (job, step.get("name")))
                self.assertNotIn("rust-toolchain", str(step.get("uses", "")), job)
        prepared = named(self.fuzz, "fuzz", "Vanilla data, and resources prepared by the built preparer")["run"]
        self.assertIn('HORIZON_PREPARE_BINARY="$RUNNER_TEMP/built/horizon-prepare" bash tools/ci/vanilla-data.sh', prepared)
        script = (ROOT / "tools/ci/vanilla-data.sh").read_text(encoding="utf-8")
        given = script.index('if [ -n "$preparer" ]; then')
        self.assertLess(given, script.index("cargo build --release --locked -p horizon-prepare"))
        self.assertIn('resource-export "$RUNNER_TEMP/prepare.log" "$preparer"', script)

    def test_only_the_slices_staged_results_are_uploaded(self):
        fuzz = steps(self.fuzz, "fuzz")
        names = [step.get("name") for step in fuzz]
        guard = names.index("Only the slices' staged results leave the runner")
        self.assertEqual(names[guard - 1], "The slices")
        self.assertEqual(names[guard + 1], "Upload the slices' results")
        self.assertEqual((fuzz[guard - 1]["id"], fuzz[guard]["id"]), ("slices", "stage_results"))
        # Not after a preparer or a smoke that failed; and no upload the check did not pass.
        self.assertEqual(fuzz[guard]["if"], "always() && steps.slices.outcome != 'skipped'")
        self.assertEqual(fuzz[guard + 1]["if"], "always() && steps.stage_results.outcome == 'success' && "
                                                "steps.stage_results.outputs.has_results == 'true'")
        self.assertEqual(fuzz[guard + 1]["with"]["path"], "${{ runner.temp }}/parity-fuzz-results/")
        self.assertNotIn("/dev/null", fuzz[guard]["run"])
        uploads = [step for job in self.fuzz["jobs"] for step in steps(self.fuzz, job)
                   if "upload-artifact" in str(step.get("uses", ""))]
        self.assertEqual(sum(step is fuzz[guard + 1] for step in uploads), 1)
        # What tools/parity_fuzz_probe.py `slices` writes, and no other name.
        probe = (ROOT / "tools/parity_fuzz_probe.py").read_text(encoding="utf-8")
        written = set(re.findall(r"kept / '([\w.-]+)'", probe)) | {"found.json", "report.json"}
        self.assertEqual(written, {"output.txt", "found.json", "report.json", "harness-tail.txt"})
        for name in written:
            self.assertIn(name.replace(".", "\\."), fuzz[guard]["run"])

        self.assertEqual(fuzz[guard]["env"], {"STAGED": "${{ steps.slices.outputs.staged }}"})
        sys.path.insert(0, str(ROOT))
        from tools import parity_fuzz_probe as probe

        def guarded(make, staged=None, after=lambda temp, out: None):
            """The step over what `make` leaves, told the digest `staged` (as the slices step computes it over that,
            when not given); `after` changes the folder once that digest is taken."""
            with tempfile.TemporaryDirectory() as folder:
                temp, out = Path(folder), Path(folder) / "parity-fuzz-results"
                (temp / "outside").mkdir()
                (temp / "outside/data.txt").write_text("data", encoding="utf-8")
                (temp / "output").touch()
                make(temp, out)
                if staged is None:
                    staged = probe.digest(out) if out.is_dir() and not out.is_symlink() else "a" * 64
                after(temp, out)
                result = subprocess.run([BASH, "-c", fuzz[guard]["run"]], capture_output=True, text=True,
                                        env={**os.environ, "RUNNER_TEMP": temp.as_posix(), "STAGED": staged,
                                             "GITHUB_OUTPUT": (temp / "output").as_posix()})
                return (result.returncode, (temp / "output").read_text(encoding="utf-8").strip(),
                        out.exists() or out.is_symlink(), (temp / "outside/data.txt").exists())

        def results(temp, out):
            for seed, files in ((1, ("output.txt", "found.json", "report.json")), (26, ("output.txt", "harness-tail.txt"))):
                (out / f"slice-{seed}").mkdir(parents=True)
                for name in files:
                    (out / f"slice-{seed}" / name).write_text("{}", encoding="utf-8")

        def plus(make):
            def made(temp, out):
                results(temp, out)
                make(temp, out)
            return made

        # The slices step stopped before it had staged: nothing is uploaded, whatever is in the folder.
        self.assertEqual(guarded(lambda temp, out: None, staged=""), (0, "has_results=false", False, True))
        self.assertEqual(guarded(results, staged=""), (0, "has_results=false", False, True))
        # It staged, and the folder is not that any more: another digest, a file changed, one added, one gone.
        self.assertEqual(guarded(results, staged="0" * 64), (1, "", False, True))
        self.assertEqual(guarded(results, staged="not a digest"), (1, "", False, True))
        for change in (lambda temp, out: (out / "slice-1/found.json").write_text("{ }", encoding="utf-8"),
                       lambda temp, out: (out / "slice-26/found.json").write_text("{}", encoding="utf-8"),
                       lambda temp, out: (out / "slice-1/report.json").unlink()):
            self.assertEqual(guarded(results, after=change), (1, "", False, True))
        self.assertEqual(guarded(lambda temp, out: None), (1, "", False, True))  # staged, and the folder is gone
        self.assertEqual(guarded(lambda temp, out: out.mkdir()), (0, "has_results=false", True, True))
        self.assertEqual(guarded(results), (0, "has_results=true", True, True))  # a lost slice's beside a result
        refused = (1, "", False, True)
        for name, make in {
            "a file of another name": lambda temp, out: (out / "slice-1/world.dat").write_text("x", encoding="utf-8"),
            "a file beside the slices": lambda temp, out: (out / "found.json").write_text("{}", encoding="utf-8"),
            "a folder of another name": lambda temp, out: (out / "data").mkdir(),
            "a folder in a slice": lambda temp, out: (out / "slice-1/batch-1").mkdir(),
            "a file as a slice": lambda temp, out: (out / "slice-7").write_text("x", encoding="utf-8"),
            "a folder as a report": lambda temp, out: (out / "slice-26/found.json").mkdir(),
        }.items():
            self.assertEqual(guarded(plus(make)), refused, name)
        self.assertEqual(guarded(lambda temp, out: out.write_text("x", encoding="utf-8")), refused)
        try:
            self.assertEqual(guarded(plus(lambda temp, out: os.symlink(temp / "outside/data.txt", out / "slice-26/report.json"))),
                             refused)
            self.assertEqual(guarded(plus(lambda temp, out: os.symlink(temp / "outside", out / "slice-9",
                                                                       target_is_directory=True))), refused)
            self.assertEqual(guarded(lambda temp, out: os.symlink(temp / "outside", out, target_is_directory=True)),
                             refused)
            self.assertEqual(guarded(plus(lambda temp, out: os.link(temp / "outside/data.txt", out / "slice-26/report.json"))),
                             refused)  # a second name of a file elsewhere
        except OSError:
            self.skipTest("this account cannot make links")

    def test_the_summary_fails_for_a_download_that_failed_or_a_planned_slice_without_a_result(self):
        self.assertEqual(self.fuzz["jobs"]["plan"]["outputs"]["slices"], "${{ steps.slices.outputs.slices }}")
        summary = steps(self.fuzz, "summary")
        download = [step for step in summary if "download-artifact" in str(step.get("uses", ""))]
        self.assertEqual([(step.get("id"), step.get("continue-on-error")) for step in download], [("download", True)])
        merge = named(self.fuzz, "summary", "The slices' groups as one")
        self.assertEqual(merge["env"], {"FUZZ": "${{ needs.fuzz.result }}", "DOWNLOAD": "${{ steps.download.outcome }}",
                                        "PLANNED": "${{ needs.plan.outputs.slices }}"})
        # The count is the merge's, of results it could count, not of files named found.json.
        self.assertIn('merge "${expect[@]}" "${found[@]}"', merge["run"])
        self.assertNotIn('"${#found[@]}" -eq "$PLANNED"', merge["run"])
        verdict = merge["run"][merge["run"].index("bad=0"):]

        def ended(merged=0, **given):
            """The step from the merge on, with a merge that ends `merged` (3: not as many whole results as planned)."""
            with tempfile.TemporaryDirectory() as folder:
                (Path(folder) / "merged").mkdir()
                script = ('python3() { echo "$*" > "$RUNNER_TEMP/asked"; echo table; return "$MERGED"; }\n'
                          'found=(a/found.json b/found.json)\n' + verdict)
                env = {**os.environ, "FUZZ": "success", "DOWNLOAD": "success", "PLANNED": "16", "MERGED": str(merged),
                       "RUNNER_TEMP": Path(folder).as_posix(), "GITHUB_STEP_SUMMARY": (Path(folder) / "summary").as_posix(),
                       **given}
                code = subprocess.run([BASH, "-c", "set -euo pipefail\n" + script], env=env, capture_output=True,
                                      text=True).returncode
                return code, (Path(folder) / "asked").read_text(encoding="utf-8").strip(), \
                    (Path(folder) / "summary").read_text(encoding="utf-8").count("table")

        asked = "tools/parity_fuzz_probe.py merge --expect 16 a/found.json b/found.json"
        self.assertEqual(ended(), (0, asked, 1))
        self.assertEqual(ended(merged=3), (1, asked, 1))  # a planned slice without a whole result: shown, and failed
        self.assertEqual(ended(DOWNLOAD="failure"), (1, asked, 1))
        self.assertEqual(ended(FUZZ="failure"), (1, asked, 1))
        for planned in ("", "0", "x"):
            self.assertEqual(ended(PLANNED=planned), (1, "tools/parity_fuzz_probe.py merge a/found.json b/found.json", 1))

    def test_every_action_is_pinned_to_a_commit(self):
        for job in self.fuzz["jobs"]:
            for step in steps(self.fuzz, job):
                if "uses" in step:
                    self.assertRegex(step["uses"], r"^[\w./-]+@[0-9a-f]{40}$")

    def test_inputs_reach_a_shell_only_as_variables(self):
        for job in self.fuzz["jobs"]:
            for step in steps(self.fuzz, job):
                self.assertNotIn("${{", step.get("run", ""), step.get("name"))

    def test_the_plan_takes_inputs_only_in_their_shapes_and_a_seed_once(self):
        run = named(self.fuzz, "plan", "The slices, by runner")["run"]
        base = {"FAMILY": "commands", "SEED": "08", "COUNT": "200", "RUNNERS": "2", "SEEDS": "", "SHRINK": "0"}

        def planned(**changed):
            with tempfile.TemporaryDirectory() as folder:
                output = Path(folder) / "output"
                output.touch()
                result = subprocess.run([BASH, "-c", run], capture_output=True, text=True, env={
                    **os.environ, **base, **changed, "GITHUB_OUTPUT": output.as_posix(),
                    "GITHUB_STEP_SUMMARY": (Path(folder) / "summary").as_posix()})
                return result.returncode, output.read_text(encoding="utf-8")

        code, output = planned()
        self.assertEqual(code, 0)
        self.assertIn('{"runner":2,"of":2,"first":1608,"seeds":""}', output)
        self.assertIn("\nslices=16\n", output)
        code, output = planned(SEEDS="7,010,9,1,2,3,4,5,6")
        self.assertEqual(code, 0)
        self.assertIn('"seeds":"7,10,9,1,2,3,4,5"},{"runner":2,"of":2,"first":0,"seeds":"6"}', output)
        self.assertIn("\nslices=9\n", output)
        # With seeds given, seed, count and runners do not count, whatever they hold.
        code, output = planned(SEEDS="3", SEED="x", COUNT="9999", RUNNERS="")
        self.assertEqual(code, 0)
        self.assertIn('"seeds":"3"}', output)
        self.assertEqual(planned(SEEDS="3", SHRINK="7"), (1, ""))
        self.assertEqual(planned(SEEDS="3", FAMILY="x"), (1, ""))
        for bad in ({"SEEDS": "1,01"}, {"SEEDS": "5,7,5"}, {"SEEDS": "1,2;x"}, {"FAMILY": "commands; id"},
                    {"COUNT": "401"}, {"COUNT": "0"}, {"RUNNERS": "9"}, {"SHRINK": "7"}, {"SEED": "-1"}):
            code, output = planned(**bad)
            self.assertEqual((code, output), (1, ""), bad)


if __name__ == "__main__":
    unittest.main()
