"""tools/parity_main.py: the base of a parity-main run, and the bisect after a failure."""
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

from tools import parity_main


class ParityMainTests(unittest.TestCase):
    def test_first_bad_finds_the_first_failing_merge(self):
        commits = list("abcdefgh")  # h failed; the base before a passed
        for culprit in commits:
            tried = []
            fails = lambda commit: tried.append(commit) or commit >= culprit
            self.assertEqual(parity_main.first_bad(commits, fails), culprit)
            self.assertLessEqual(len(tried), 3, "log2 of 8 merges")
        self.assertEqual(parity_main.first_bad(["a"], lambda commit: self.fail("nothing to try")), "a")

    def test_base_is_the_newest_green_run_head_contains(self):
        with tempfile.TemporaryDirectory() as tmp, mock.patch.object(parity_main, "ROOT", Path(tmp)):
            git = lambda *arguments: parity_main.git(*arguments).stdout.strip()
            git("init", "-q")
            for n in range(3):
                git("-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", str(n))
            head, parent, grandparent = git("rev-parse", "HEAD"), git("rev-parse", "HEAD^1"), git("rev-parse", "HEAD~2")
            git("checkout", "-q", "-b", "side", grandparent)
            git("-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "side")
            side = git("rev-parse", "HEAD")
            git("checkout", "-q", head)
            self.assertEqual(parity_main.base(runs=[side, grandparent, parent]), grandparent,
                             "a run on a commit HEAD doesn't contain is skipped")
            self.assertEqual(parity_main.base(runs=[]), parent, "no green run yet: the last merge")
            self.assertEqual(parity_main.base(runs=[head]), head)

    def bisect(self, broken_from, merges=4, budgets=None, scripted=None, late=None, known=None):
        """Run bisect over merges where each harness of broken_from fails from that merge's index on (None: never;
        0: at the base too). Returns the exit code and the pull requests commented on; with budgets, one run after
        another on the same runner, each as (exit code, comments, commits it ran harnesses at, what it reported).
        A harness of scripted fails as its list says, one entry a run, wherever it runs; the budget is spent once late() says so."""
        with tempfile.TemporaryDirectory() as tmp, mock.patch.object(parity_main, "ROOT", Path(tmp)):
            git = lambda *arguments: parity_main.git(*arguments).stdout.strip()
            git("init", "-q")
            for n in range(merges):
                git("-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty",
                    "-m", f"Merge pull request #{n} from x/y")
            commits = git("rev-list", "--reverse", "HEAD").split()
            last_run = Path(tmp) / ".local/parity-runner/last-run.json"
            last_run.parent.mkdir(parents=True)
            last_run.write_text(json.dumps({"failed": list(broken_from)}))
            if known:
                (Path(tmp) / "tools").mkdir()
                (Path(tmp) / "tools/parity_hosts.json").write_text(json.dumps({"known_failing": known}))
            real_run = parity_main.subprocess.run
            comments, ran, reported = [], [], []
            # git runs for real in the scratch repo; a `gh pr comment` is only recorded, never sent.
            fake_run = lambda command, **kwargs: (comments.append(command) or mock.Mock(returncode=0)
                                                  if command[0] == "gh" else real_run(command, **kwargs))
            at = lambda: commits.index(git("rev-parse", "HEAD"))
            run = lambda names, timeout: ran.append(at()) or [
                name for name in names if (scripted[name].pop(0) if name in (scripted or {}) else
                                           broken_from[name] is not None and at() >= broken_from[name])]
            runs = []
            with mock.patch.object(parity_main, "LAST_RUN", last_run), \
                 mock.patch.object(parity_main, "BISECT", last_run.with_name("bisect.json")), \
                 mock.patch.object(parity_main, "run_harnesses", run), \
                 mock.patch.object(parity_main.subprocess, "run", fake_run),                  mock.patch.object(parity_main.time, "monotonic", lambda: 1e9 if late and late() else 0.0), \
                 mock.patch("builtins.print", lambda *lines, **kwargs: reported.extend(lines)):
                for budget in budgets or [90]:
                    del comments[:], ran[:], reported[:]
                    code = parity_main.bisect(commits[0], 60, comment=True, budget=budget)
                    self.assertEqual(git("rev-parse", "HEAD"), commits[-1], "HEAD checked out again")
                    runs.append((code, [(command[3], command[-1].split("breaks ")[1].split(" on main")[0])
                                        for command in comments], list(ran), [line[len("parity-main: "):]
                                        for line in reported if line.startswith("parity-main: ")]))
            return runs if budgets else runs[0][:2]

    def test_bisect_names_a_merge_only_when_head_fails_and_the_base_passes(self):
        self.assertEqual(self.bisect({"music": None}), (0, []), "0 only when the failures pass again at HEAD")
        self.assertEqual(self.bisect({"music": 0}), (1, []), "failing at the base too blames nobody")
        self.assertEqual(self.bisect({"music": 2}), (1, [("2", "`music`")]))

    def test_bisect_judges_each_harness_on_its_own(self):
        # One harness failing at the base too doesn't hide the merge that broke the others (parity-main 10-31).
        self.assertEqual(self.bisect({"lunge": 0, "music": 2, "flaky": None, "sound": 2}),
                         (1, [("2", "`music`, `sound`")]))
        # Two merges broke two harnesses: each is named.
        self.assertEqual(self.bisect({"music": 3, "sound": 1}), (1, [("1", "`sound`"), ("3", "`music`")]))

    def test_known_failing_harnesses_are_not_searched(self):
        # parity.py passes a probe whose only failure is a known failing harness (tools/parity_hosts.json), so a
        # search for one would read passes where it fails and blame whichever merge broke the others.
        self.assertEqual(self.bisect({"datafix#2": 1, "crafter": 1, "mace": 2},
                                     known={"datafix": "#1: paths", "crafter": "#2: max_damage"}),
                         (1, [("2", "`mace`")]))
        self.assertEqual(self.bisect({"datafix#2": 1, "datafix#1": 2}, known={"datafix#2": "#1: paths"}),
                         (1, [("2", "`datafix#1`")]), "a listed shard leaves its harness's other shards searched")

    def test_a_search_out_of_time_says_how_far_it_got_and_the_next_run_goes_on(self):
        # 2026-11-09 to 11-11: seven runs searched up to 1,500 merges until the job's limit, and named nothing.
        first, second, third = self.bisect({"music": 11}, merges=16, budgets=[0, 90, 90])
        code, comments, ran, reported = first
        self.assertEqual((code, comments, ran), (1, [], [15, 0]), "HEAD and the base, then no time for the search")
        self.assertIn("ended with 15 merges left. The next run goes on from there.", reported[0])
        code, comments, ran, _ = second
        self.assertEqual((code, comments), (1, [("11", "`music`")]))
        self.assertEqual(ran[:1], [15], "HEAD runs again")
        self.assertNotIn(0, ran, "the base is not run again")
        code, comments, ran, reported = third
        self.assertEqual((code, comments, ran), (1, [], [15]), "a merge already named is not commented on again")
        self.assertIn("music first fail at", reported[0])

    def test_a_search_goes_on_between_the_merges_an_earlier_one_tried(self):
        # Finding sound's merge (2) tried merges 7, 3 and 1, where music passed: music's search starts above 7.
        code, comments, ran, _ = self.bisect({"sound": 2, "music": 11}, merges=16, budgets=[90])[0]
        self.assertEqual(comments, [("2", "`sound`"), ("11", "`music`")])
        self.assertEqual(ran, [15, 0, 7, 3, 1, 2, 2, 1, 11, 9, 10, 11, 10], "each named merge and the one before run again")

    def test_a_harness_that_fails_some_of_the_time_names_no_merge(self):
        # 2026-11-15: pearl_wind_charge failed twice at HEAD and passed once at the base and at each merge tried, so
        # the search ended on #16690; it fails one run in three before that merge too.
        fail, ok = True, False
        for runs, why in (([fail, ok, ok, ok, fail, fail], "fails at the commit before too"),
                          ([fail, ok, ok, ok, ok], "passes at the merge")):
            code, comments, ran, reported = self.bisect({"music": 3, "pearl": None}, scripted={"pearl": runs},
                                                        budgets=[90])[0]
            self.assertEqual((code, comments), (1, [("3", "`music`")]), why)
            self.assertEqual(runs, [], "the merge, then the commit before it, ran again")
            self.assertTrue(any("pearl" in line and "flaky" in line for line in reported), reported)
        # The budget ends between the two reruns: the one that passed at the merge is settled, the other goes on.
        runs = [fail, ok, ok, ok, ok]
        code, comments, _, reported = self.bisect({"music": 3, "pearl": None}, scripted={"pearl": runs},
                                                  late=lambda: not runs, budgets=[90])[0]
        self.assertEqual((code, comments), (1, []))
        self.assertTrue(any("pearl" in line and "flaky" in line for line in reported), reported)
        self.assertTrue(any("music passes at" in line and "The next run goes on" in line for line in reported), reported)
        # Failing again at the merge and passing again before it names the merge.
        self.assertEqual(self.bisect({"pearl": None}, scripted={"pearl": [fail, ok, ok, ok, fail, ok]}),
                         (1, [("3", "`pearl`")]))

    def run_command(self, command, last_run, fails=None, run_id=None, known=None, flaky=None, since_head=False,
                    records=None, history=None, folders=(), rerun_exit=None):
        """parity_main.py COMMAND BASE through main(), in a fresh scratch repo of 3 merges on a fresh runner folder.
        last_run: last-run.json (None: missing, str: its raw text). fails: {harness: "head" (fails there only),
        "always" or None}. records: state.json's harnesses before the rerun. folders: runner files that are folders
        instead, so reading or writing them fails. rerun_exit: run_harnesses runs for real, and tools/parity.py exits
        with it without recording anything (a rerun that stopped). Returns (exit code, commits harnesses ran at, lines
        reported, this run's verdict report or None, the flake history after)."""
        fails = fails or {}
        with tempfile.TemporaryDirectory() as tmp, mock.patch.object(parity_main, "ROOT", Path(tmp)):
            git = lambda *arguments: parity_main.git(*arguments).stdout.strip()
            git("init", "-q")
            for n in range(3):
                git("-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty",
                    "-m", f"Merge pull request #{n} from x/y")
            commits = git("rev-list", "--reverse", "HEAD").split()
            runner = Path(tmp) / ".local/parity-runner"
            runner.mkdir(parents=True)
            for name in folders:
                (runner / name).mkdir()
            if last_run is not None:
                (runner / "last-run.json").write_text(last_run if isinstance(last_run, str) else json.dumps(last_run))
            if records is not None:
                (runner / "state.json").write_text(json.dumps({"harnesses": records}))
            if history is not None:
                (runner / "verdict-history.json").write_text(history if isinstance(history, str) else json.dumps(history))
            (Path(tmp) / "tools").mkdir()
            (Path(tmp) / "tools/parity_hosts.json").write_text(json.dumps({"known_failing": known or {},
                                                                           "known_flaky": flaky or {}}))
            real_run = parity_main.subprocess.run
            ran, reported = [], []
            at = lambda: commits.index(git("rev-parse", "HEAD"))
            for name in (last_run or {}).get("failed", []) if rerun_exit is not None else []:
                (Path(tmp) / f"tools/test_{name}_parity.py").write_text("")

            def fake_run(command, **kwargs):
                if command[0] == "gh":
                    return mock.Mock(returncode=0)
                if command[1:2] == ["tools/parity.py"]:
                    ran.append(at())
                    return mock.Mock(returncode=rerun_exit)
                return real_run(command, **kwargs)

            def run(names, timeout):
                ran.append(at())
                return [name for name in names if fails.get(name) == "always" or
                        fails.get(name) == "head" and at() == len(commits) - 1]
            environment = {key: value for key, value in parity_main.os.environ.items()
                           if key not in ("GITHUB_STEP_SUMMARY", "PARITY_RUN_ID")}
            environment.update({"PARITY_RUN_ID": run_id} if run_id else {})
            since = commits[-1] if since_head else commits[0]
            with mock.patch.object(parity_main, "LAST_RUN", runner / "last-run.json"), \
                 mock.patch.object(parity_main, "BISECT", runner / "bisect.json"), \
                 mock.patch.object(parity_main, "STATE", runner / "state.json"), \
                 mock.patch.object(parity_main, "VERDICT", runner / "verdict.json"), \
                 mock.patch.object(parity_main, "HISTORY", runner / "verdict-history.json"), \
                 mock.patch.object(parity_main, "run_harnesses", parity_main.run_harnesses if rerun_exit is not None
                                   else run), \
                 mock.patch.object(parity_main.subprocess, "run", fake_run), \
                 mock.patch.dict(parity_main.os.environ, environment, clear=True), \
                 mock.patch.object(parity_main.sys, "argv", ["parity_main.py", command] + [since] * (command != "history")), \
                 mock.patch("builtins.print", lambda *lines, **kwargs: reported.extend(map(str, lines))):
                code = parity_main.main()
            read = lambda name: json.loads((runner / name).read_text()) if (runner / name).is_file() else None
            report_name = f"verdict-{run_id}.json" if run_id else "verdict.json"
            return code, ran, reported, read(report_name), read("verdict-history.json")

    def test_verdict_exits_as_bisect_does(self):
        # The gate runs `verdict`; `bisect` (by hand) must reach the same verdict, or one of them judges wrongly.
        cases = {
            "no failure": ({"failed": []}, {}, {}),
            "every failure passes again": ({"failed": ["music", "sound"]}, {}, {}),
            "one fails again at HEAD only": ({"failed": ["music", "sound"]}, {"sound": "head"}, {}),
            "one fails again everywhere": ({"failed": ["music"]}, {"music": "always"}, {}),
            "only known failing ones": ({"failed": ["datafix#2", "crafter"]}, {"crafter": "always"},
                                        {"known": {"datafix": "#1", "crafter": "#2"}}),
            "last-run.json missing": (None, {}, {}),
            "last-run.json unreadable": ("{not json", {}, {}),
            "last-run.json not an object": ("[]", {}, {}),
            "no merge since the base": ({"failed": ["music"]}, {}, {"since_head": True}),
            "this run's list": ({"run": "7-1", "failed": ["music"]}, {}, {"run_id": "7-1"}),
            "an earlier run's list": ({"run": "6-1", "failed": ["music"]}, {}, {"run_id": "7-1"}),
            "a list without a run": ({"failed": ["music"]}, {}, {"run_id": "7-1"}),
            "by hand, any list": ({"run": "6-1", "failed": ["music"]}, {}, {}),
        }
        passing = {"every failure passes again", "this run's list", "by hand, any list"}
        rerun = passing | {"one fails again at HEAD only", "one fails again everywhere"}
        for case, (last_run, fails, options) in cases.items():
            with self.subTest(case):
                code, ran, _, _, _ = self.run_command("verdict", last_run, fails, **options)
                self.assertEqual(code, 0 if case in passing else 1)
                self.assertEqual(self.run_command("bisect", last_run, fails, **options)[0], code)
                self.assertEqual(ran, [2] if case in rerun else [],
                                 "verdict runs the failed harnesses again at HEAD only, and only with a list to judge")

    def test_a_list_an_earlier_run_left_fails_the_run(self):
        # tools/parity.py writes last-run.json only when its run ends: one that stopped before leaves the last list, a
        # flaky harness's that passes at HEAD, and the batch nothing judged would pass.
        for command in ("verdict", "bisect"):
            code, ran, reported, _, _ = self.run_command(command, {"run": "6-1", "failed": ["music"]}, run_id="7-1")
            self.assertEqual((code, ran), (1, []), command)
            self.assertTrue(any("not this one (7-1)" in line for line in reported), reported)

    def test_verdict_json_holds_each_try_and_the_repeat_flakers(self):
        records = {"music": {"status": "fail"}, "pearl": {"status": "fail", "retried": "known flaky"}}
        code, _, reported, written, history = self.run_command(
            "verdict", {"run": "11-2", "failed": ["music", "pearl", "sound"]}, {"sound": "always"}, run_id="11-2",
            records=records, flaky={"pearl": "#5: heap"},
            history=[{"run": f"{n}-1", "head": "x", "rerun_passed": ["music", "pearl"]} for n in range(1, 11)])
        self.assertEqual(code, 1)
        self.assertEqual(written["exit_code"], 1)
        self.assertEqual(written["harnesses"], {
            "music": [{"try": "batch", "result": "fail"}, {"try": "rerun at HEAD", "result": "pass"}],
            "pearl": [{"try": "batch", "result": "fail"}, {"try": "batch, retried (known flaky)", "result": "fail"},
                      {"try": "rerun at HEAD", "result": "pass"}],
            "sound": [{"try": "batch", "result": "fail"}, {"try": "rerun at HEAD", "result": "fail"}]})
        self.assertEqual(written["repeat_flakers"], ["music"], "pearl is on known_flaky")
        self.assertEqual([run["run"] for run in history], [f"{n}-1" for n in range(2, 11)] + ["11-2"], "the last 10")
        self.assertEqual(sum("repeat flaker" in line for line in reported), 1, reported)
        # This run's rerun and two earlier ones of the last ten make a repeat flaker; one that dropped out of the ten
        # doesn't count. A passing verdict writes its file too.
        needed, clean = {"head": "x", "rerun_passed": ["music"]}, {"head": "x", "rerun_passed": []}
        earlier = [dict(needed, run="1-1"), dict(needed, run="2-1")] + [dict(clean, run=f"{n}-1") for n in range(3, 10)]
        code, _, _, written, _ = self.run_command("verdict", {"run": "10-1", "failed": ["music"]}, run_id="10-1",
                                                  history=earlier)
        self.assertEqual((code, written["exit_code"], written["repeat_flakers"]), (0, 0, ["music"]))
        code, _, _, written, _ = self.run_command("verdict", {"run": "11-1", "failed": ["music"]}, run_id="11-1",
                                                  history=earlier + [dict(clean, run="10-1")])
        self.assertEqual((code, written["repeat_flakers"]), (0, []))
        # A history that can't be read starts again; bisect by hand keeps none.
        code, _, _, written, history = self.run_command("verdict", {"run": "3-1", "failed": ["music"]},
                                                        run_id="3-1", history="{half")
        self.assertEqual((code, [run["run"] for run in history]), (0, ["3-1"]))
        self.assertIsNone(self.run_command("bisect", {"run": "3-1", "failed": ["music"]}, run_id="3-1")[4])

    def test_what_failed_the_old_bisect_fails_both_commands(self):
        # The same command checking itself can't see a change from before: these pin the old bisect's effects.
        # The record of the rerun at HEAD (bisect.json) can't be written: red, as it was, though the harness passed.
        for command in ("verdict", "bisect"):
            with self.subTest(command), self.assertRaises(OSError):
                self.run_command(command, {"failed": ["music"]}, folders=["bisect.json"])
        # last-run.json that exists and can't be read (not only malformed): red, judged on nothing.
        for command in ("verdict", "bisect"):
            self.assertEqual(self.run_command(command, None, folders=["last-run.json"])[:2], (1, []), command)
        # The rerun stopped (tools/parity.py exits nonzero and records nothing new): the batch's list stands, red.
        for command in ("verdict", "bisect"):
            for exit_code in (1, -9):
                code, ran, _, _, _ = self.run_command(command, {"run": "5-1", "failed": ["music"]}, run_id="5-1",
                                                      rerun_exit=exit_code)
                self.assertEqual((code, ran[:1]), (1, [2]), (command, exit_code))
            self.assertEqual(self.run_command(command, {"run": "5-1", "failed": ["music"]}, run_id="5-1",
                                              rerun_exit=0)[0], 0, "and passes when the rerun does")

    def test_the_flake_history_and_the_report_never_change_the_verdict(self):
        last_run = {"run": "5-1", "failed": ["music", "sound"]}
        for case, options in {
            "history is a folder": {"folders": ["verdict-history.json"]},
            "malformed history entries": {"history": [{"run": "1-1", "rerun_passed": 5}, "junk", [],
                                                      {"run": "2-1", "rerun_passed": [["x"], None]}]},
            "an old report that is a folder": {"folders": ["verdict-5-1.json"]},
        }.items():
            for fails, expected in (({}, 0), ({"sound": "always"}, 1)):
                with self.subTest(case, fails=fails):
                    code, ran, _, written, _ = self.run_command("verdict", last_run, fails, run_id="5-1", **options)
                    self.assertEqual((code, ran), (expected, [2]))
                    if "report" not in case:
                        self.assertEqual((written["exit_code"], sorted(written["harnesses"])),
                                         (expected, ["music", "sound"]), "the report is written all the same")

    def test_a_run_whose_harnesses_passed_counts_in_the_history(self):
        code, ran, _, _, history = self.run_command("history", None, run_id="4-1",
                                                    history=[{"run": "3-1", "head": "x", "rerun_passed": ["music"]}])
        self.assertEqual((code, ran, [(run["run"], run["rerun_passed"]) for run in history]),
                         (0, [], [("3-1", ["music"]), ("4-1", [])]))
        with self.assertRaises(SystemExit):
            self.run_command("history", None)

    def test_the_workflow_gives_the_verdict_and_leaves_the_search_out(self):
        root = Path(__file__).resolve().parents[1]
        # The repository's offline YAML parser, as tests/test_ci_concurrency.py uses it.
        with mock.patch("sys.path", [str(root / "tools/ci/vendor/pyyaml-6.0.3.zip"), *sys.path]):
            import yaml
        workflow = yaml.safe_load((root / ".github/workflows/parity-main.yml").read_text(encoding="utf-8"))
        job = workflow["jobs"]["parity-main"]
        self.assertEqual(job["env"]["PARITY_RUN_ID"], "${{ github.run_id }}-${{ github.run_attempt }}")
        commands = [step["run"].split() for step in job["steps"] if "run" in step]
        self.assertIn(["python3", "tools/parity_main.py", "verdict", '"$BASE_SHA"'], commands)
        self.assertFalse([command for command in commands if "bisect" in command], "the search is not in the gate run")
        verdict = next(step for step in job["steps"] if "verdict" in step.get("run", ""))
        self.assertEqual(verdict.get("timeout-minutes"), 50)
        self.assertNotIn("continue-on-error", verdict, "the verdict step decides the run")
        upload = next(step for step in job["steps"] if step.get("name") == "Upload the verdict")
        self.assertTrue(upload["continue-on-error"], "the artifact service never decides the run")
        self.assertEqual(upload["with"]["path"],
                         ".local/parity-runner/verdict-${{ github.run_id }}-${{ github.run_attempt }}.json")

    def test_a_harness_a_commit_does_not_have_cannot_fail_there(self):
        with mock.patch.object(parity_main.subprocess, "run") as run:
            self.assertEqual(parity_main.run_harnesses(["not_a_harness_yet"], 60), [])
            run.assert_not_called()

    def test_only_parity_mains_harness_step_keeps_the_prebuild_timings(self):
        root = Path(__file__).resolve().parents[1]
        with mock.patch("sys.path", [str(root / "tools/ci/vendor/pyyaml-6.0.3.zip"), *sys.path]):
            import yaml
        switch = "HORIZON_PARITY_PREBUILD_TIMINGS"
        setting = [path.name for path in (root / ".github").rglob("*") if path.is_file()
                   and switch in path.read_text(encoding="utf-8", errors="replace")]
        self.assertEqual(setting, ["parity-main.yml"], "PR parity, nightly and parity-hosted upload nothing")
        job = yaml.safe_load((root / ".github/workflows/parity-main.yml").read_text(encoding="utf-8"))["jobs"]["parity-main"]
        names = [step.get("name") for step in job["steps"]]
        harnesses = names.index("The harnesses the merges since then can affect")
        self.assertEqual([index for index, step in enumerate(job["steps"]) if switch in step.get("env", {})],
                         [harnesses], "the verdict's reruns build without it")
        self.assertNotIn(switch, job["env"])
        self.assertEqual(job["steps"][harnesses]["env"][switch], "1")
        index = names.index("Upload the prebuild's timing reports")
        upload = job["steps"][index]
        # Right after the harnesses: the verdict steps that follow run only on a failure.
        self.assertEqual(index, harnesses + 1)
        self.assertLess(index, min(i for i, name in enumerate(names) if name and "verdict" in name))
        # Only when the harness step's tools/parity.py says it published, and then this attempt's folder (which
        # tools/parity.py names after PARITY_RUN_ID), never a path from the output, which cargo and the harnesses
        # can write too; in it only the two kinds of file it holds.
        self.assertEqual(job["steps"][harnesses]["id"], "harnesses")
        folder = f".local/parity-runner/prebuild-timings-{job['env']['PARITY_RUN_ID']}"
        self.assertEqual(upload["if"], "${{ !cancelled() && steps.harnesses.outputs.prebuild_timings == 'ready' }}")
        self.assertNotIn("outputs", upload["with"]["path"])
        self.assertTrue(upload["continue-on-error"], "the artifact service never decides the run")
        self.assertEqual((upload["with"]["retention-days"], upload["with"]["if-no-files-found"]), (7, "ignore"))
        self.assertEqual(upload["with"]["path"].splitlines(), [f"{folder}/cargo-*.html", f"{folder}/prebuild-calls.txt"])
        self.assertEqual(upload["uses"], job["steps"][names.index("Upload the verdict")]["uses"])


if __name__ == "__main__":
    unittest.main()
