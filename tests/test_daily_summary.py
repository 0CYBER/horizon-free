"""The daily summary: the daily-green tag, one issue per failing test, and nothing written outside main."""
import contextlib
import io
import json
import os
import re
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from tools.ci import daily_summary as summary

COMMIT = "c" * 40
LEGS = {leg: {"result": "success"} for leg in summary.LEGS}


def job(name, conclusion="success", failed=()):
    steps = [{"name": "Set up job", "conclusion": "success"}] + [{"name": step, "conclusion": "failure"} for step in failed]
    return {"name": name, "conclusion": conclusion, "steps": steps}


GREEN_JOBS = [job("shards / plan"), job("shards / ignored-tests-shards (horizon-world)"),
              job("wasm-mods / fixture"), job("wasm-mods / Guest stack / ubuntu-latest"),
              job("fn-replace-check / route"), job("fn-replace-check / fn-replace-check"),
              job("linkage / route"), job("linkage / linkage x86_64-unknown-linux-gnu"),
              job("windows-runtime-leases / gate"), job("windows-runtime-leases / test", "skipped"),
              job("windows-runtime-leases / runtime-leases"), job("windows-runtime-leases / launcher-processes"),
              job("machine-sample / route"), job("machine-sample / build"),
              job("sdk / fresh"), job("sdk / build linux"), job("sdk / publish", "skipped")]
# Every other shard of the daily matrix, green (the summary expects each by name: summary.expected_shards).
GREEN_JOBS += [job(name) for name in summary.expected_shards() if name != "shards / ignored-tests-shards (horizon-world)"]


def shard_report(problems=(), passed=(), incomplete=()):
    return {"shards": ["horizon-world"], "problems": list(problems), "passed": list(passed),
            "incomplete": list(incomplete)}


def failure(test, panic="crates/horizon-world/src/lib.rs:7:3", kind="failed", package="horizon-world"):
    return {"package": package, "test": test, "kind": kind, "panic": panic, "policy": None}


class Api:
    """GitHub's REST API as the summary sees it: listings by page, and every write recorded."""

    def __init__(self, jobs, issues=(), tag=False, fail_on=None, comments=None, runs=None):
        self.jobs, self.issues, self.tag, self.fail_on, self.writes = jobs, list(issues), tag, fail_on, []
        self.comments = comments if comments is not None else {}
        self.runs = runs or {}

    def __call__(self, method, path, body=None):
        if self.fail_on and self.fail_on in path:
            raise SystemExit("::error::GitHub refused GET: HTTP 403 (rate limit exhausted)")
        if method != "GET":
            self.writes.append((method, path, body))
            if path.endswith("/comments"):
                self.comments.setdefault(int(path.split("/")[1]), []).append(body["body"])
            elif path == "issues":
                self.issues.append({"number": 1000 + len(self.issues), "title": body["title"], "body": body["body"]})
            return {}
        if "matching-refs" in path:
            return [{"ref": "refs/tags/daily-green"}] if self.tag else []
        if re.fullmatch(r"issues/\d+", path):
            number = int(path.split("/")[1])
            return {"body": None, **next(issue for issue in self.issues if issue["number"] == number)}
        if "workflows/daily.yml/runs" in path:
            return {"workflow_runs": list(self.runs.values())} if path.endswith("page=1") else {"workflow_runs": []}
        if re.search(r"actions/runs/(\d+)/jobs", path) and "/runs/77/" not in path:
            ident = int(re.search(r"actions/runs/(\d+)/jobs", path)[1])
            return {"jobs": self.runs[ident]["jobs"]} if path.endswith("page=1") else {"jobs": []}
        page = int(path.rsplit("page=", 1)[1])
        if re.fullmatch(r"issues/\d+/comments\?per_page=100&page=\d+", path):
            found = self.comments.get(int(path.split("/")[1]), [])
            return [{"body": text} for text in found[(page - 1) * 100:page * 100]]
        if "matching-refs" in path:
            return [{"ref": "refs/tags/daily-green"}] if self.tag else []
        page = int(path.rsplit("page=", 1)[1])
        if "/jobs" in path:
            return {"jobs": self.jobs[(page - 1) * 100:page * 100]}
        if path.startswith("issues?"):
            closed = "state=closed" in path
            listed = [issue for issue in self.issues if (issue.get("state") == "closed") == closed]
            return listed[(page - 1) * 100:page * 100]
        raise AssertionError(path)


def closers(fixes):
    """GraphQL as fixes_after sees it: issue number -> the merge commit of the pull request that closed it."""
    def query(text, variables):
        fix = fixes.get(variables["number"])
        nodes = [{"closer": {"mergeCommit": {"oid": fix}} if fix else None}]
        return {"repository": {"issue": {"timelineItems": {"nodes": nodes}}}}
    return query


def git(merged=(), tag=True, ancestors=()):
    def run(*args):
        if args[0] == "rev-parse":
            return "d" * 40 if tag else None
        if args[0] == "merge-base":
            return "" if args[2] in ancestors else None
        return "\n".join(f"Merge pull request #{n} from arismoko/topic" for n in merged) + "\nMerge branch 'x'"
    return run


class DailySummaryTests(unittest.TestCase):
    def run_summary(self, api, reports=(), results=LEGS, ref="refs/heads/main", dry_run="false", merged=(), tag=True,
                    fixes=None, ancestors=()):
        with tempfile.TemporaryDirectory() as folder:
            for index, report in enumerate(reports):
                path = Path(folder, "reports", f"daily-report-{index}")
                path.mkdir(parents=True)
                (path / "daily-report.json").write_text(json.dumps(report))
            env = {"GITHUB_REPOSITORY": "arismoko/horizon", "GITHUB_RUN_ID": "77", "GITHUB_REF": ref,
                   "GITHUB_SERVER_URL": "https://github.com", "COMMIT": COMMIT, "DRY_RUN": dry_run,
                   "RESULTS": json.dumps(results), "REPORTS": str(Path(folder, "reports")),
                   "GITHUB_STEP_SUMMARY": str(Path(folder, "summary")), "GITHUB_OUTPUT": str(Path(folder, "out"))}
            with mock.patch.dict(os.environ, env, clear=True), contextlib.redirect_stdout(io.StringIO()):
                summary.main(api, git(merged, tag, ancestors), closers(fixes or {}))
            return Path(folder, "summary").read_text(), Path(folder, "out").read_text()

    def run_after(self, api, result, ref="refs/heads/main", run="77", fixes=None):
        env = {"GITHUB_REPOSITORY": "arismoko/horizon", "GITHUB_RUN_ID": run, "GITHUB_REF": ref,
               "GITHUB_SERVER_URL": "https://github.com", "COMMIT": COMMIT, "DRY_RUN": "false"}
        with mock.patch.dict(os.environ, env, clear=True), contextlib.redirect_stdout(io.StringIO()):
            summary.after("sdk-publish", result, api, git((), True), closers(fixes or {}))

    def test_a_failed_sdk_publish_files_one_issue_and_its_next_success_closes_it(self):
        failed = [job("sdk-publish / index"), job("sdk-publish / publish", "failure", ["Release sdk-<sha>"])]
        api = Api(failed)
        self.run_after(api, "failure")
        created = [body for method, path, body in api.writes if method == "POST" and path == "issues"]
        self.assertEqual([body["title"] for body in created], ["Daily: sdk-publish did not complete"])
        self.assertIn("`sdk-publish / publish: Release sdk-<sha>`", created[0]["body"])
        # A later run's failure comments on that issue instead of filing another.
        api.writes.clear()
        api.runs = {78: {"jobs": failed}}
        self.run_after(api, "failure", run="78")
        self.assertEqual([(method, path) for method, path, _ in api.writes], [("POST", "issues/1000/comments")])
        api.writes.clear()
        self.run_after(api, "success", run="79")
        self.assertIn(("PATCH", "issues/1000", {"state": "closed", "state_reason": "completed"}), api.writes)
        # A failure on a commit that predates the fix which closed the issue comments on it and leaves it closed.
        api.issues[0]["state"] = "closed"
        api.runs[80] = {"jobs": failed}
        api.writes.clear()
        self.run_after(api, "failure", run="80", fixes={1000: "f" * 40})
        self.assertEqual([(method, path) for method, path, _ in api.writes], [("POST", "issues/1000/comments")])
        self.assertIn("which predates the fix `fffffffff`", api.writes[0][2]["body"])
        # Off main nothing is written.
        quiet = Api(failed)
        self.run_after(quiet, "failure", ref="refs/heads/topic")
        self.assertEqual(quiet.writes, [])

    def test_all_green_moves_the_tag_and_closes_issues_that_went_green(self):
        api = Api(GREEN_JOBS, issues=[{"number": 5, "title": "Daily: horizon-world a::b"},
                                      {"number": 6, "title": "Daily: fn-replace-check / fn-replace-check did not complete"},
                                      {"number": 7, "title": "Daily: horizon-world still::unseen"},
                                      {"number": 8, "title": "Something else"}], tag=True)
        text, out = self.run_summary(api, [shard_report(passed=[{"package": "horizon-world", "test": "a::b"}])])
        self.assertIn("green=true", out)
        self.assertIn(("PATCH", "git/refs/tags/daily-green", {"sha": COMMIT, "force": True}), api.writes)
        closed = [path for method, path, body in api.writes if method == "PATCH" and path.startswith("issues/")]
        self.assertCountEqual(closed, ["issues/5", "issues/6"])
        self.assertFalse(any(path.startswith("issues/7") for _, path, _ in api.writes))
        self.assertIn("Move `daily-green`", text)

    def test_a_skipped_sdk_build_or_publish_is_fine_but_a_skipped_shard_is_not(self):
        jobs = [job for job in GREEN_JOBS if not job["name"].startswith("sdk")] + [
            job("sdk / fresh"), job("sdk / build linux", "skipped"), job("sdk / publish", "skipped")]
        _, out = self.run_summary(Api(jobs), [shard_report()])
        self.assertIn("green=true", out, "sdk-<sha> already exists, and a dry run publishes nothing")
        jobs = [job("shards / ignored-tests-shards (horizon-world)", "skipped")] + GREEN_JOBS[2:]
        api = Api(jobs)
        _, out = self.run_summary(api, [])
        self.assertIn("green=false", out, "a skipped shard is missing coverage")

    def test_a_failed_sdk_build_is_red(self):
        jobs = [job for job in GREEN_JOBS if not job["name"].startswith("sdk")] + [
            job("sdk / fresh"), job("sdk / build windows", "failure", ["Build and collect"])]
        api = Api(jobs)
        _, out = self.run_summary(api, [shard_report()], results=dict(LEGS, sdk={"result": "failure"}))
        self.assertIn("green=false", out)
        self.assertEqual([body["title"] for _, path, body in api.writes if path == "issues"],
                         ["Daily: sdk / build windows did not complete"])

    def test_the_tag_is_created_the_first_time(self):
        api = Api(GREEN_JOBS, tag=False)
        self.run_summary(api, [shard_report()])
        self.assertIn(("POST", "git/refs", {"ref": "refs/tags/daily-green", "sha": COMMIT}), api.writes)

    def test_a_failing_test_gets_one_issue_with_location_run_and_suspects(self):
        jobs = GREEN_JOBS[:1] + [job("shards / ignored-tests-shards (horizon-world)", "failure",
                                     ["Run the ignored tests that need prepared data"])] + GREEN_JOBS[2:]
        api = Api(jobs)
        report = shard_report([failure("game::x::tests::broken"), failure("game::x::tests::broken")])
        _, out = self.run_summary(api, [report], results=dict(LEGS, shards={"result": "failure"}), merged=[12, 15])
        self.assertIn("green=false", out)
        created = [body for method, path, body in api.writes if (method, path) == ("POST", "issues")]
        self.assertEqual(len(created), 1, "one issue per failing test")
        issue = created[0]
        self.assertEqual(issue["title"], "Daily: horizon-world game::x::tests::broken")
        self.assertEqual(issue["labels"], ["bug"])
        self.assertTrue(issue["body"].startswith(summary.DISCLOSURE + "\n"))
        for part in ("`crates/horizon-world/src/lib.rs:7:3`", "https://github.com/arismoko/horizon/actions/runs/77",
                     "#12, #15"):
            self.assertIn(part, issue["body"])
        self.assertFalse(any("tags" in path for _, path, _ in api.writes), "a red run never moves the tag")

    def test_an_open_issue_gets_a_comment_not_a_duplicate(self):
        jobs = [job("shards / ignored-tests-shards (horizon-world)", "failure", ["Run"])] + GREEN_JOBS[2:]
        issues = [{"number": n, "title": f"filler {n}"} for n in range(150)]
        issues.append({"number": 900, "title": "Daily: horizon-world game::x::tests::broken", "body": "> filed"})
        api = Api(jobs, issues=issues)
        self.run_summary(api, [shard_report([failure("game::x::tests::broken")])],
                         results=dict(LEGS, shards={"result": "failure"}))
        self.assertEqual([(m, p) for m, p, _ in api.writes], [("POST", "issues/900/comments")],
                         "found past the first page of open issues")
        self.assertTrue(api.writes[0][2]["body"].startswith("🤖 Agent: "))

    def test_a_red_whose_fix_merged_after_the_tested_commit_comments_on_the_closed_issue(self):
        jobs = [job("shards / ignored-tests-shards (horizon-world)", "failure", ["Run"])] + GREEN_JOBS[2:]
        late, early = "f" * 40, "e" * 40
        issues = [{"number": 901, "title": "Daily: horizon-world a::fixed_later", "state": "closed"},
                  {"number": 902, "title": "Daily: horizon-world a::fixed_before", "state": "closed"},
                  {"number": 903, "title": "Daily: horizon-world a::closed_by_hand", "state": "closed"}]
        api = Api(jobs, issues=issues)
        report = shard_report([failure("a::fixed_later"), failure("a::fixed_before"), failure("a::closed_by_hand")])
        self.run_summary(api, [report], results=dict(LEGS, shards={"result": "failure"}),
                         fixes={901: late, 902: early}, ancestors=(early,))
        comments = [(path, body["body"]) for method, path, body in api.writes if path.endswith("/comments")]
        self.assertEqual([path for path, _ in comments], ["issues/901/comments"])
        self.assertIn(f"on `{COMMIT[:9]}`, which predates the fix `{late[:9]}`", comments[0][1])
        self.assertFalse(any(path == "issues/901" for _, path, _ in api.writes), "it stays closed")
        # A fix the tested commit holds did not fix it, and a hand close names no fix: both are filed anew.
        self.assertCountEqual([body["title"] for _, path, body in api.writes if path == "issues"],
                              ["Daily: horizon-world a::fixed_before", "Daily: horizon-world a::closed_by_hand"])

    def test_a_leg_that_failed_without_test_names_gets_one_issue_per_leg(self):
        jobs = [job("shards / ignored-tests-shards (horizon-world)", "cancelled", ["Prepare checkout-keyed vanilla data"]),
                job("wasm-mods / fixture", "failure", ["Build tool and shared member"]),
                job("fn-replace-check / fn-replace-check", "failure", ["Refusals by name"])] + GREEN_JOBS[6:]
        api = Api(jobs)
        self.run_summary(api, [], results=dict(LEGS, shards={"result": "cancelled"}, **{"wasm-mods": {"result": "failure"}},
                                               **{"fn-replace-check": {"result": "failure"}}), tag=False)
        titles = sorted(body["title"] for method, path, body in api.writes if path == "issues")
        self.assertEqual(titles, ["Daily: fn-replace-check / fn-replace-check did not complete",
                                  "Daily: shards / ignored-tests-shards (horizon-world) did not complete",
                                  "Daily: wasm-mods / fixture did not complete"])
        bodies = " ".join(body["body"] for _, path, body in api.writes if path == "issues")
        self.assertIn("`Refusals by name`", bodies)
        self.assertIn("no daily run has been green yet", bodies)

    def test_a_shard_killed_for_memory_says_by_what_with_its_memory_time_and_running_tests(self):
        stuck = [f"game::slow::case_{n}" for n in range(7)]
        incomplete = {"shard": "horizon-world", "reason": "a test process crashed", "ended_by": "signal 9 (SIGKILL)",
                      "cargo_exit": 101, "wall_seconds": 4454, "memory_peak_bytes": 34359738368,
                      "memory_max_bytes": 34359738368, "oom_kills": 1, "still_running": stuck,
                      "still_running_more": 0, "never_reported": None}
        jobs = [job("shards / ignored-tests-shards (horizon-world)", "failure",
                    ["Run the ignored tests that need prepared data"])] + GREEN_JOBS[2:]
        api = Api(jobs)
        self.run_summary(api, [shard_report(incomplete=[incomplete])], results=dict(LEGS, shards={"result": "failure"}))
        body = next(body["body"] for _, path, body in api.writes if path == "issues")
        self.assertIn("a test run did not complete: `horizon-world` killed by signal 9 (SIGKILL), out of memory "
                      "(1 kill by the kernel); job memory peak 32.0 GiB of 32.0 GiB; after 74 min; still running past "
                      "60 s: `game::slow::case_0`, `game::slow::case_1`, `game::slow::case_2`, `game::slow::case_3`, "
                      "`game::slow::case_4` and 2 more.", body)

    def test_what_a_stopped_run_reports_is_shown_only_as_names_and_numbers(self):
        self.assertEqual(summary.stopped_text({"shard": "horizon-client#2of2", "in_progress": True}),
                         "`horizon-client#2of2` was still running when the leg ended (its time limit, a cancel, or a "
                         "lost runner); which tests were running and the process's end are not known")
        self.assertEqual(summary.stopped_text({"shard": "x", "reason": "listed tests never reported a result",
                                               "cargo_exit": 0, "wall_seconds": 30, "never_reported": 2}),
                         "`x`: listed tests never reported a result (cargo exit 0); after 30 s; "
                         "2 listed tests never reported")
        text = summary.stopped_text({"shard": "x`<b>", "reason": "PRIVATE: payload", "ended_by": "signal 9; rm",
                                     "still_running": ["ok::name", "not a name", "ghp_x y"], "wall_seconds": "9",
                                     "memory_peak_bytes": True, "oom_kills": -1})
        self.assertEqual(text, "a run did not complete; still running past 60 s: `ok::name`")
        self.assertEqual(summary.stopped_text({"shard": "x"}), "`x` did not complete")

    def test_every_shard_of_the_matrix_must_report_by_name(self):
        launcher = "shards / ignored-tests-shards (daily-rows-launcher)"
        self.assertIn(launcher, summary.expected_shards())
        _, out = self.run_summary(Api(GREEN_JOBS), [shard_report()])
        self.assertIn("green=true", out)
        # Every other job green, the launcher's shard absent: red, with one leg issue naming it.
        api = Api([entry for entry in GREEN_JOBS if entry["name"] != launcher])
        _, out = self.run_summary(api, [shard_report()])
        self.assertIn("green=false", out)
        self.assertEqual([body["title"] for _, path, body in api.writes if path == "issues"],
                         [f"Daily: {launcher} did not complete"])
        # A shard renamed in the workflow but not in the matrix's source: the expected name is missing.
        renamed = [job("shards / ignored-tests-shards (daily-rows-tools)") if entry["name"] == launcher else entry
                   for entry in GREEN_JOBS]
        _, out = self.run_summary(Api(renamed), [shard_report()])
        self.assertIn("green=false", out)

    def test_a_stopped_runs_test_names_are_bounded_in_length(self):
        long = "a" * 301
        text = summary.stopped_text({"shard": "x", "still_running": [long, "ok::name"]})
        self.assertEqual(text, "`x` did not complete; still running past 60 s: `ok::name`")
        self.assertIn("a" * 300, summary.stopped_text({"shard": "x", "still_running": ["a" * 300]}))

    def test_a_skipped_leg_with_no_jobs_is_not_green(self):
        api = Api([job for job in GREEN_JOBS if not job["name"].startswith("shards")])
        _, out = self.run_summary(api, [], results=dict(LEGS, shards={"result": "skipped"}))
        self.assertIn("green=false", out)
        self.assertIn("Daily: shards did not complete", [body["title"] for _, path, body in api.writes if path == "issues"])

    def test_fixture_test_failures_are_filed_by_test_name(self):
        jobs = GREEN_JOBS[:2] + [job("wasm-mods / fixture", "failure", [summary.FIXTURE_TESTS_STEP])] + GREEN_JOBS[3:]
        report = {"needs": "HORIZON_WASM_FIXTURE", "incomplete": [], "passed": [],
                  "problems": [failure("game::wasm::wasm_interaction_shared_matrix", None, package="horizon-client")]}
        api = Api(jobs)
        self.run_summary(api, [report], results=dict(LEGS, **{"wasm-mods": {"result": "failure"}}))
        titles = [body["title"] for _, path, body in api.writes if path == "issues"]
        self.assertEqual(titles, ["Daily: horizon-client game::wasm::wasm_interaction_shared_matrix"])

    def test_a_second_invocation_for_one_run_posts_nothing_twice(self):
        jobs = [job("shards / ignored-tests-shards (horizon-world)", "failure", ["Run"])] + GREEN_JOBS[2:]
        red = dict(LEGS, shards={"result": "failure"})
        issues = [{"number": 900, "title": "Daily: horizon-world old::red", "body": "> filed earlier"},
                  {"number": 901, "title": "Daily: horizon-world was::red", "body": "> filed earlier"}]
        report = shard_report([failure("old::red"), failure("new::red")],
                              passed=[{"package": "horizon-world", "test": "was::red"}])
        api = Api(jobs, issues=issues)
        self.run_summary(api, [report], results=red)
        first = list(api.writes)
        self.assertEqual(sorted((m, p) for m, p, _ in first),
                         [("PATCH", "issues/901"), ("POST", "issues"), ("POST", "issues/900/comments"),
                          ("POST", "issues/901/comments")])
        api.writes.clear()
        # The rerun sees the issue it filed (its body names this run) and the comments it posted.
        self.run_summary(api, [report], results=red)
        self.assertEqual([(m, p) for m, p, _ in api.writes], [("PATCH", "issues/901")],
                         "only the idempotent close is repeated")

    def test_a_partial_first_application_completes_on_the_second(self):
        jobs = [job("shards / ignored-tests-shards (horizon-world)", "failure", ["Run"])] + GREEN_JOBS[2:]
        run_url = "https://github.com/arismoko/horizon/actions/runs/77"
        # The first attempt filed the new issue, then stopped before commenting on the old one.
        issues = [{"number": 900, "title": "Daily: horizon-world old::red", "body": "> filed earlier"},
                  {"number": 950, "title": "Daily: horizon-world new::red", "body": f"> filed\nFirst daily run that saw it: {run_url} on `ccc`."}]
        api = Api(jobs, issues=issues)
        self.run_summary(api, [shard_report([failure("old::red"), failure("new::red")])],
                         results=dict(LEGS, shards={"result": "failure"}))
        self.assertEqual([(m, p) for m, p, _ in api.writes], [("POST", "issues/900/comments")])

    def test_a_failed_helper_files_one_leg_issue_naming_it(self):
        for helper, leg, skipped in (("sdk / fresh", "sdk", ["sdk / build linux"]),
                                     ("fn-replace-check / route", "fn-replace-check", ["fn-replace-check / fn-replace-check"])):
            with self.subTest(helper=helper):
                jobs = [j for j in GREEN_JOBS if not j["name"].startswith(leg + " / ")]
                jobs += [job(helper, "failure", ["Set up job"])] + [job(name, "skipped") for name in skipped]
                api = Api(jobs)
                _, out = self.run_summary(api, [shard_report()], results=dict(LEGS, **{leg: {"result": "failure"}}))
                self.assertIn("green=false", out)
                created = [body for _, path, body in api.writes if path == "issues"]
                self.assertEqual([body["title"] for body in created], [f"Daily: {leg} did not complete"])
                self.assertIn(f"`{helper}` failed", created[0]["body"])

    def test_a_red_machine_sample_files_one_action_issue_naming_the_step_and_closes_when_green(self):
        step = "The machine sample builds against this API"
        jobs = [j for j in GREEN_JOBS if j["name"] != "machine-sample / build"] + [
            job("machine-sample / build", "failure", [step])]
        api = Api(jobs)
        _, out = self.run_summary(api, [shard_report()], results=dict(LEGS, **{"machine-sample": {"result": "failure"}}),
                                  merged=[4100, 4101])
        self.assertIn("green=false", out)
        created = [body for _, path, body in api.writes if path == "issues"]
        self.assertEqual([body["title"] for body in created], ["Daily: update the machine sample for the mod API change"])
        self.assertEqual(created[0]["labels"], ["bug"])
        self.assertIn(f"`{step}`", created[0]["body"])
        self.assertIn("#4100, #4101", created[0]["body"])
        self.assertIn("check_machine_sample.py", created[0]["body"])
        # Still red the next day: a comment on the same issue. Green: closed.
        issue = [{"number": 77, "title": "Daily: update the machine sample for the mod API change", "body": "> filed"}]
        api = Api(jobs, issues=issue)
        self.run_summary(api, [shard_report()], results=dict(LEGS, **{"machine-sample": {"result": "failure"}}))
        self.assertEqual([(m, p) for m, p, _ in api.writes], [("POST", "issues/77/comments")])
        api = Api(GREEN_JOBS, issues=issue)
        _, out = self.run_summary(api, [shard_report()])
        self.assertIn("green=true", out)
        self.assertIn(("PATCH", "issues/77"), [(m, p) for m, p, _ in api.writes])

    def test_a_skipped_sdk_build_is_green_only_when_fresh_found_the_release(self):
        jobs = [j for j in GREEN_JOBS if not j["name"].startswith("sdk / ")] + [
            job("sdk / fresh", "cancelled"), job("sdk / build linux", "skipped")]
        api = Api(jobs)
        _, out = self.run_summary(api, [shard_report()], results=dict(LEGS, sdk={"result": "cancelled"}))
        self.assertIn("green=false", out)
        self.assertEqual([body["title"] for _, path, body in api.writes if path == "issues"],
                         ["Daily: sdk did not complete"])

    def test_a_rerun_that_turns_a_test_green_still_posts_its_close_comment(self):
        red_jobs = [job("shards / ignored-tests-shards (horizon-world)", "failure", ["Run"])] + GREEN_JOBS[2:]
        issues = [{"number": 900, "title": "Daily: horizon-world flaky::one", "body": "> filed earlier"}]
        api = Api(red_jobs, issues=issues)
        self.run_summary(api, [shard_report([failure("flaky::one")])], results=dict(LEGS, shards={"result": "failure"}))
        self.assertEqual([(m, p) for m, p, _ in api.writes], [("POST", "issues/900/comments")])
        api.writes.clear()
        api.jobs = GREEN_JOBS
        self.run_summary(api, [shard_report(passed=[{"package": "horizon-world", "test": "flaky::one"}])])
        self.assertEqual([(m, p) for m, p, _ in api.writes if p.startswith("issues/")],
                         [("POST", "issues/900/comments"), ("PATCH", "issues/900")],
                         "the red comment of this run does not hide its green-again comment")
        comment = next(body for _, path, body in api.writes if path == "issues/900/comments")
        self.assertIn("Green again in the daily run", comment["body"])

    def test_a_leg_level_issue_closes_once_the_leg_passes_whole(self):
        failed = [j for j in GREEN_JOBS if not j["name"].startswith("sdk / ")] + [
            job("sdk / fresh", "failure", ["Set up job"]), job("sdk / build linux", "skipped")]
        api = Api(failed)
        self.run_summary(api, [shard_report()], results=dict(LEGS, sdk={"result": "failure"}))
        created = [body["title"] for _, path, body in api.writes if path == "issues"]
        self.assertEqual(created, ["Daily: sdk did not complete"])
        number = api.issues[-1]["number"]
        api.writes.clear()
        api.jobs = [j for j in GREEN_JOBS if not j["name"].startswith("sdk / ")]
        self.run_summary(api, [shard_report()], results=dict(LEGS, sdk={"result": "success"}))
        self.assertNotIn(("PATCH", f"issues/{number}"), [(m, p) for m, p, _ in api.writes],
                         "a run where the leg did not report leaves it open")
        api.writes.clear()
        api.jobs = GREEN_JOBS
        self.run_summary(api, [shard_report()])
        self.assertIn(("PATCH", f"issues/{number}"), [(m, p) for m, p, _ in api.writes])
        comment = next(body for _, path, body in api.writes if path == f"issues/{number}/comments")
        self.assertIn("Green again in the daily run", comment["body"])

    def test_an_unknown_job_or_a_leg_with_no_job_is_red(self):
        for jobs, title in (
                (GREEN_JOBS + [job("surprise / something")], "Daily: surprise / something did not complete"),
                ([j for j in GREEN_JOBS if not j["name"].startswith("linkage")], "Daily: linkage did not complete")):
            with self.subTest(title=title):
                api = Api(jobs)
                _, out = self.run_summary(api, [shard_report()])
                self.assertIn("green=false", out, "never silently green")
                self.assertEqual([body["title"] for _, path, body in api.writes if path == "issues"], [title])
                self.assertIn("did not report", api.writes[0][2]["body"])

    def test_seen_finds_a_completed_real_run_by_its_summary_job(self):
        sha = "e" * 40
        runs = {5: {"id": 5, "conclusion": "failure", "html_url": "u5", "jobs": [job(f"summary {sha}", "failure")]},
                6: {"id": 6, "conclusion": "success", "html_url": "u6", "jobs": [job(f"summary {sha} (dry run)")]},
                7: {"id": 7, "conclusion": "cancelled", "html_url": "u7", "jobs": [job(f"summary {sha}")]}}
        with mock.patch.dict(os.environ, {"GITHUB_RUN_ID": "77", "GITHUB_REPOSITORY": "arismoko/horizon"}):
            self.assertEqual(summary.seen(sha, Api([], runs=runs)), "u5", "a failed real run counts")
            del runs[5]
            self.assertEqual(summary.seen(sha, Api([], runs=runs)), "", "dry and cancelled runs do not")

    def test_nothing_is_written_off_main_or_in_a_dry_run(self):
        jobs = [job("shards / ignored-tests-shards (horizon-world)", "failure", ["Run"])] + GREEN_JOBS[2:]
        for ref, dry_run in (("refs/heads/hotspots-daily", "false"), ("refs/heads/main", "true")):
            with self.subTest(ref=ref, dry_run=dry_run):
                api = Api(jobs)
                text, _ = self.run_summary(api, [shard_report([failure("a::b")])], ref=ref, dry_run=dry_run,
                                           results=dict(LEGS, shards={"result": "failure"}))
                self.assertEqual(api.writes, [])
                self.assertIn("(dry run: nothing written)", text)
                self.assertIn("File **Daily: horizon-world a::b**", text)

    def test_a_failed_lookup_writes_nothing(self):
        jobs = [job("shards / ignored-tests-shards (horizon-world)", "failure", ["Run"])] + GREEN_JOBS[2:]
        api = Api(jobs, fail_on="issues?")
        with self.assertRaises(SystemExit):
            self.run_summary(api, [shard_report([failure("a::b")])], results=dict(LEGS, shards={"result": "failure"}))
        self.assertEqual(api.writes, [])


class DailyWorkflowTests(unittest.TestCase):
    def setUp(self):
        from tests.test_ci_concurrency import ROOT, yaml
        self.workflow = yaml.load((ROOT / ".github/workflows/daily.yml").read_text(), Loader=yaml.BaseLoader)

    def test_the_schedule_only_dispatches_a_real_run_on_main(self):
        jobs = self.workflow["jobs"]
        self.assertEqual(self.workflow["on"]["schedule"], [{"cron": "0 9 * * *"}])
        self.assertIn("github.event_name == 'schedule'", jobs["schedule-dispatch"]["if"])
        self.assertEqual(jobs["schedule-dispatch"]["runs-on"], "ubuntu-latest")
        self.assertIn("--ref main -f dry_run=false -f scheduled=true", jobs["schedule-dispatch"]["steps"][0]["run"])
        resolve = jobs["resolve"]["steps"][-1]
        self.assertIn('if [ "$SCHEDULED" = true ] && [ "$DRY_RUN" != true ]', resolve["run"],
                      "only the schedule stops on a commit already tested; a hand dispatch retries")
        self.assertEqual(self.workflow["on"]["workflow_dispatch"]["inputs"]["scheduled"]["default"], "false")
        self.assertEqual(jobs["resolve"]["if"], "github.event_name == 'workflow_dispatch'")
        self.assertEqual(self.workflow["on"]["workflow_dispatch"]["inputs"]["dry_run"]["default"], "true")

    def test_only_main_without_dry_run_writes(self):
        jobs = self.workflow["jobs"]
        step = jobs["resolve"]["steps"][-1]
        self.assertEqual(step["env"]["DRY_RUN"], "${{ inputs.dry_run || github.ref != 'refs/heads/main' }}")
        # A writing run takes only a commit of main: an unmerged commit is never tagged, signed or published.
        self.assertIn('git merge-base --is-ancestor "$sha" refs/remotes/origin/main', step["run"])
        self.assertEqual(jobs["summary"]["steps"][-1]["env"]["DRY_RUN"], "${{ needs.resolve.outputs.dry_run }}")
        self.assertEqual(jobs["nightly"]["if"],
                         "always() && needs.summary.outputs.green == 'true' && needs.resolve.outputs.dry_run == 'false'"
                         " && (needs.sdk-publish.result == 'success' || needs.sdk-publish.result == 'skipped')")
        self.assertEqual(self.workflow["concurrency"]["cancel-in-progress"], "false")
        self.assertEqual(jobs["sdk"]["with"], {"commit": "${{ needs.resolve.outputs.commit }}"})
        self.assertNotIn("permissions", jobs["sdk"], "the build leg publishes nothing")
        publish = jobs["sdk-publish"]
        self.assertEqual(publish["needs"], ["resolve", "summary", "sdk"])
        self.assertEqual(publish["if"], "needs.summary.outputs.green == 'true' && needs.resolve.outputs.dry_run == 'false'"
                                        " && needs.sdk.outputs.needed == 'true'")
        # The one publish step, signed only on main through the called workflow's `release` environment.
        self.assertEqual(publish["uses"], "./.github/workflows/sdk-publish.yml")
        self.assertEqual(publish["with"], {"commit": "${{ needs.resolve.outputs.commit }}", "publish": "true"})
        self.assertIn("sdk-publish", jobs["nightly"]["needs"])
        report = jobs["sdk-publish-report"]
        self.assertIn("sdk-publish", report["needs"])
        self.assertIn("daily_summary.py --after sdk-publish", report["steps"][-1]["run"])
        self.assertEqual(report["steps"][-1]["env"]["DRY_RUN"], "${{ needs.resolve.outputs.dry_run }}")

        self.assertEqual(jobs["nightly"]["with"]["daily"], "true")
        self.assertTrue(jobs["summary"]["name"].startswith("summary ${{ needs.resolve.outputs.commit }}"))

    def test_the_sdk_build_packs_exactly_the_archives_a_release_needs(self):
        from tests.test_ci_concurrency import ROOT, yaml
        from tools.ci import nightly
        build = yaml.load((ROOT / ".github/workflows/prebuilt-game-build.yml").read_text(),
                          Loader=yaml.BaseLoader)["jobs"]["build"]
        # Each matrix leg packs its three slices; the build data and surface are gathered once from them all.
        names = {f"{kind}-{entry['target']}{'-server' if entry.get('server') == 'true' else ''}.tar.xz"
                 for entry in build["strategy"]["matrix"]["include"] for kind in ("runtime", "api", "engine")}
        self.assertEqual(names | {"api-source.tar.xz", "build-data.tar.xz", "surface.json"}, set(nightly.SDK_ARCHIVES))
        self.assertEqual(len(nightly.SDK_ARCHIVES), len(set(nightly.SDK_ARCHIVES)))
        steps = {step.get("name") or step.get("uses"): step for step in build["steps"]}
        self.assertEqual(steps["Pack the SDK's assets"]["run"], "python tools/prebuilt_game.py pack prebuilt dist")
        self.assertEqual(steps["actions/upload-artifact@v4"]["with"]["name"], "sdk-leg-${{ matrix.name }}")
        self.assertNotIn("env", build, "no hand-set archive name")

        # The launcher and the mod tool ask for horizon_mod_package::sdk::Asset::name(kind, target, side):
        # `{kind}-{target}{-server}.tar.xz`, the kinds named runtime, api and engine; and the release's other files
        # are its BESIDE_ASSETS. Producer (the legs), publish check (SDK_ARCHIVES, SDK_ASSETS) and launcher agree.
        sdk = (ROOT / "crates/horizon-mod-package/src/sdk.rs").read_text()
        self.assertIn('format!("{}-{target}{suffix}.tar.xz", kind.name())', sdk)
        self.assertIn('let suffix = if side == Side::Server { "-server" } else { "" };', sdk)
        for kind in ("runtime", "api", "engine", "build-data", "api-source"):
            self.assertIn(f'=> "{kind}",', sdk)
        self.assertIn('pub const INDEX: &str = "sdk.json";', sdk)
        self.assertIn('pub const BESIDE_ASSETS: [&str; 3] = [INDEX, "SHA256SUMS", "SHA256SUMS.sig"];', sdk)
        self.assertEqual(set(nightly.SDK_ASSETS) - set(nightly.SDK_ARCHIVES), {"sdk.json", "SHA256SUMS", "SHA256SUMS.sig"})

    def test_the_sdk_is_signed_and_published_only_on_main(self):
        from tests.test_ci_concurrency import ROOT, yaml
        workflow = yaml.load((ROOT / ".github/workflows/sdk-publish.yml").read_text(), Loader=yaml.BaseLoader)
        jobs = workflow["jobs"]
        on_main = "inputs.publish && github.ref == 'refs/heads/main'"
        self.assertEqual(jobs["sign"]["environment"], f"${{{{ {on_main} && 'release' || '' }}}}")
        sign = jobs["sign"]["steps"][-3]
        self.assertEqual(sign["run"], 'bash tools/ci/sign_release.sh "$TAG" dist/*')
        self.assertEqual(sign["env"]["EPHEMERAL"], f"${{{{ !({on_main}) && 'yes' || '' }}}}")
        for key in ("RELEASE_SIGNING_KEY", "RELEASE_SIGNING_KEY_NEXT"):
            self.assertEqual(sign["env"][key], f"${{{{ {on_main} && secrets.{key} || '' }}}}")
        self.assertEqual(workflow["env"]["TAG"], "sdk-${{ inputs.commit }}")
        # Only the sign job sees the key, and it checks out nothing but the signing script; the key signs only a
        # commit of main, whatever commit the caller names.
        self.assertEqual(jobs["sign"]["steps"][0]["with"]["sparse-checkout"], "tools/ci/sign_release.sh")
        on_main_check = jobs["sign"]["steps"][1]
        self.assertEqual(on_main_check["if"], f"${{{{ {on_main} }}}}")
        self.assertIn('git merge-base --is-ancestor "$COMMIT" refs/remotes/origin/main', on_main_check["run"])
        # The jobs that read the legs' output cannot write; the legs' artifacts only, not an earlier attempt's own.
        for job in ("index", "sign"):
            self.assertEqual(jobs[job]["permissions"], {"contents": "read"}, job)
        self.assertEqual(jobs["publish"]["permissions"], {"contents": "write"})
        self.assertEqual(jobs["index"]["steps"][4]["with"]["pattern"], "sdk-leg-*")
        self.assertNotIn("environment", jobs["index"])
        self.assertNotIn("environment", jobs["publish"])
        publish = {step.get("name"): step for step in jobs["publish"]["steps"]}
        self.assertEqual(jobs["publish"]["env"]["PUBLISH"], f"${{{{ {on_main} }}}}")
        self.assertEqual(publish["Release sdk-<sha>"]["if"], "env.PUBLISH == 'true'")
        order = [step.get("name") for step in jobs["publish"]["steps"]]
        self.assertLess(order.index("The signature, checked as the launcher checks it"), order.index("Release sdk-<sha>"))
        self.assertLess(order.index("Exactly the assets a Nightly needs"), order.index("Release sdk-<sha>"))
        hand = yaml.load((ROOT / ".github/workflows/prebuilt-game.yml").read_text(), Loader=yaml.BaseLoader)
        self.assertEqual(hand["jobs"]["publish"]["with"]["publish"], "${{ github.ref == 'refs/heads/main' }}")

    def test_every_leg_is_a_called_workflow_with_the_commit(self):
        jobs = self.workflow["jobs"]
        for leg in summary.LEGS:
            with self.subTest(leg=leg):
                self.assertTrue(jobs[leg]["uses"].startswith("./.github/workflows/"))
                self.assertEqual(jobs[leg]["with"]["commit"], "${{ needs.resolve.outputs.commit }}")
                self.assertIn(leg, jobs["summary"]["needs"])


if __name__ == "__main__":
    unittest.main()
