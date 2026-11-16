"""The daily summary: one issue per failing test, and nothing written outside main."""
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
              job("wasm-mods / fixture", "skipped"), job("wasm-mods / Guest stack / ubuntu-latest"),
              job("fn-replace-check / route"), job("fn-replace-check / fn-replace-check"),
              job("linkage / route"), job("linkage / linkage x86_64-unknown-linux-gnu"),
              job("windows-runtime-leases / gate"), job("windows-runtime-leases / test", "skipped"),
              job("windows-runtime-leases / runtime-leases"), job("windows-runtime-leases / launcher-processes")]
# Every other shard of the daily matrix, green (the summary expects each by name: summary.expected_shards).
GREEN_JOBS += [job(name) for name in summary.expected_shards() if name != "shards / ignored-tests-shards (horizon-world)"]


def shard_report(problems=(), passed=(), incomplete=()):
    return {"shards": ["horizon-world"], "problems": list(problems), "passed": list(passed),
            "incomplete": list(incomplete)}


def failure(test, panic="crates/horizon-world/src/lib.rs:7:3", kind="failed", package="horizon-world"):
    return {"package": package, "test": test, "kind": kind, "panic": panic, "policy": None}


class Api:
    """GitHub's REST API as the summary sees it: listings by page, and every write recorded."""

    def __init__(self, jobs, issues=(), fail_on=None, comments=None, runs=None):
        self.jobs, self.issues, self.fail_on, self.writes = jobs, list(issues), fail_on, []
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


def git(ancestors=()):
    """`git merge-base --is-ancestor` as fixes_after runs it: success for the commits in `ancestors`."""
    def run(*args):
        assert args[:2] == ("merge-base", "--is-ancestor"), args
        return "" if args[2] in ancestors else None
    return run


class DailySummaryTests(unittest.TestCase):
    def run_summary(self, api, reports=(), results=LEGS, ref="refs/heads/main", dry_run="false", fixes=None,
                    ancestors=()):
        with tempfile.TemporaryDirectory() as folder:
            for index, report in enumerate(reports):
                path = Path(folder, "reports", f"daily-report-{index}")
                path.mkdir(parents=True)
                (path / "daily-report.json").write_text(json.dumps(report))
            env = {"GITHUB_REPOSITORY": "horizon-rust/horizon", "GITHUB_RUN_ID": "77", "GITHUB_REF": ref,
                   "GITHUB_SERVER_URL": "https://github.com", "COMMIT": COMMIT, "DRY_RUN": dry_run,
                   "RESULTS": json.dumps(results), "REPORTS": str(Path(folder, "reports")),
                   "GITHUB_STEP_SUMMARY": str(Path(folder, "summary"))}
            with mock.patch.dict(os.environ, env, clear=True), contextlib.redirect_stdout(io.StringIO()):
                summary.main(api, git(ancestors), closers(fixes or {}))
            return Path(folder, "summary").read_text()

    def test_all_green_closes_issues_that_went_green(self):
        api = Api(GREEN_JOBS, issues=[{"number": 5, "title": "Daily: horizon-world a::b"},
                                      {"number": 6, "title": "Daily: fn-replace-check / fn-replace-check did not complete"},
                                      {"number": 7, "title": "Daily: horizon-world still::unseen"},
                                      {"number": 8, "title": "Something else"}])
        self.run_summary(api, [shard_report(passed=[{"package": "horizon-world", "test": "a::b"}])])
        closed = [path for method, path, body in api.writes if method == "PATCH" and path.startswith("issues/")]
        self.assertCountEqual(closed, ["issues/5", "issues/6"])
        self.assertFalse(any(path.startswith("issues/7") for _, path, _ in api.writes))

    def test_a_skipped_shard_is_missing_coverage(self):
        jobs = [job("shards / ignored-tests-shards (horizon-world)", "skipped")] + GREEN_JOBS[2:]
        api = Api(jobs)
        self.run_summary(api, [])
        self.assertIn("Daily: shards / ignored-tests-shards (horizon-world) did not complete",
                      [body["title"] for _, path, body in api.writes if path == "issues"])

    def test_a_failing_test_gets_one_issue_with_location_and_run(self):
        jobs = GREEN_JOBS[:1] + [job("shards / ignored-tests-shards (horizon-world)", "failure",
                                     ["Run the ignored tests that need prepared data"])] + GREEN_JOBS[2:]
        api = Api(jobs)
        report = shard_report([failure("game::x::tests::broken"), failure("game::x::tests::broken")])
        self.run_summary(api, [report], results=dict(LEGS, shards={"result": "failure"}))
        created = [body for method, path, body in api.writes if (method, path) == ("POST", "issues")]
        self.assertEqual(len(created), 1, "one issue per failing test")
        issue = created[0]
        self.assertEqual(issue["title"], "Daily: horizon-world game::x::tests::broken")
        self.assertEqual(issue["labels"], ["bug"])
        self.assertTrue(issue["body"].startswith(summary.DISCLOSURE + "\n"))
        for part in ("`crates/horizon-world/src/lib.rs:7:3`", "https://github.com/horizon-rust/horizon/actions/runs/77"):
            self.assertIn(part, issue["body"])

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
        jobs = [job("shards / ignored-tests-shards (horizon-world)", "cancelled", ["Run the ignored tests that need no vanilla data"]),
                job("wasm-mods / Guest stack / ubuntu-latest", "failure", ["Build tool and shared member"]),
                job("fn-replace-check / fn-replace-check", "failure", ["Refusals by name"])] + GREEN_JOBS[6:]
        api = Api(jobs)
        self.run_summary(api, [], results=dict(LEGS, shards={"result": "cancelled"}, **{"wasm-mods": {"result": "failure"}},
                                               **{"fn-replace-check": {"result": "failure"}}))
        titles = sorted(body["title"] for method, path, body in api.writes if path == "issues")
        self.assertEqual(titles, ["Daily: fn-replace-check / fn-replace-check did not complete",
                                  "Daily: shards / ignored-tests-shards (horizon-world) did not complete",
                                  "Daily: wasm-mods / Guest stack / ubuntu-latest did not complete"])
        bodies = " ".join(body["body"] for _, path, body in api.writes if path == "issues")
        self.assertIn("`Refusals by name`", bodies)

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
        api = Api(GREEN_JOBS)
        self.run_summary(api, [shard_report()])
        self.assertEqual(api.writes, [])
        # Every other job green, the launcher's shard absent: red, with one leg issue naming it.
        api = Api([entry for entry in GREEN_JOBS if entry["name"] != launcher])
        self.run_summary(api, [shard_report()])
        self.assertEqual([body["title"] for _, path, body in api.writes if path == "issues"],
                         [f"Daily: {launcher} did not complete"])
        # A shard renamed in the workflow but not in the matrix's source: the expected name is missing.
        renamed = [job("shards / ignored-tests-shards (daily-rows-tools)") if entry["name"] == launcher else entry
                   for entry in GREEN_JOBS]
        api = Api(renamed)
        self.run_summary(api, [shard_report()])
        self.assertEqual([body["title"] for _, path, body in api.writes if path == "issues"],
                         [f"Daily: {launcher} did not complete"])

    def test_a_stopped_runs_test_names_are_bounded_in_length(self):
        long = "a" * 301
        text = summary.stopped_text({"shard": "x", "still_running": [long, "ok::name"]})
        self.assertEqual(text, "`x` did not complete; still running past 60 s: `ok::name`")
        self.assertIn("a" * 300, summary.stopped_text({"shard": "x", "still_running": ["a" * 300]}))

    def test_a_skipped_leg_with_no_jobs_is_not_green(self):
        api = Api([job for job in GREEN_JOBS if not job["name"].startswith("shards")])
        self.run_summary(api, [], results=dict(LEGS, shards={"result": "skipped"}))
        self.assertIn("Daily: shards did not complete", [body["title"] for _, path, body in api.writes if path == "issues"])

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
        run_url = "https://github.com/horizon-rust/horizon/actions/runs/77"
        # The first attempt filed the new issue, then stopped before commenting on the old one.
        issues = [{"number": 900, "title": "Daily: horizon-world old::red", "body": "> filed earlier"},
                  {"number": 950, "title": "Daily: horizon-world new::red", "body": f"> filed\nFirst daily run that saw it: {run_url} on `ccc`."}]
        api = Api(jobs, issues=issues)
        self.run_summary(api, [shard_report([failure("old::red"), failure("new::red")])],
                         results=dict(LEGS, shards={"result": "failure"}))
        self.assertEqual([(m, p) for m, p, _ in api.writes], [("POST", "issues/900/comments")])

    def test_a_failed_helper_files_one_leg_issue_naming_it(self):
        for helper, leg, skipped in (("fn-replace-check / route", "fn-replace-check", ["fn-replace-check / fn-replace-check"]),
                                     ("linkage / route", "linkage", ["linkage / linkage x86_64-unknown-linux-gnu"])):
            with self.subTest(helper=helper):
                jobs = [j for j in GREEN_JOBS if not j["name"].startswith(leg + " / ")]
                jobs += [job(helper, "failure", ["Set up job"])] + [job(name, "skipped") for name in skipped]
                api = Api(jobs)
                self.run_summary(api, [shard_report()], results=dict(LEGS, **{leg: {"result": "failure"}}))
                created = [body for _, path, body in api.writes if path == "issues"]
                self.assertEqual([body["title"] for body in created], [f"Daily: {leg} did not complete"])
                self.assertIn(f"`{helper}` failed", created[0]["body"])

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
        rest = [j for j in GREEN_JOBS if not j["name"].startswith("linkage / ")]
        failed = rest + [job("linkage / route", "failure", ["Set up job"]), job("linkage / linkage x86_64-unknown-linux-gnu", "skipped")]
        api = Api(failed)
        self.run_summary(api, [shard_report()], results=dict(LEGS, linkage={"result": "failure"}))
        created = [body["title"] for _, path, body in api.writes if path == "issues"]
        self.assertEqual(created, ["Daily: linkage did not complete"])
        number = api.issues[-1]["number"]
        api.writes.clear()
        api.jobs = rest
        self.run_summary(api, [shard_report()], results=dict(LEGS, linkage={"result": "success"}))
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
                self.run_summary(api, [shard_report()])
                self.assertEqual([body["title"] for _, path, body in api.writes if path == "issues"], [title])
                self.assertIn("did not report", api.writes[0][2]["body"])

    def test_seen_finds_a_completed_real_run_by_its_summary_job(self):
        sha = "e" * 40
        runs = {5: {"id": 5, "conclusion": "failure", "html_url": "u5", "jobs": [job(f"summary {sha}", "failure")]},
                6: {"id": 6, "conclusion": "success", "html_url": "u6", "jobs": [job(f"summary {sha} (dry run)")]},
                7: {"id": 7, "conclusion": "cancelled", "html_url": "u7", "jobs": [job(f"summary {sha}")]}}
        with mock.patch.dict(os.environ, {"GITHUB_RUN_ID": "77", "GITHUB_REPOSITORY": "horizon-rust/horizon"}):
            self.assertEqual(summary.seen(sha, Api([], runs=runs)), "u5", "a failed real run counts")
            del runs[5]
            self.assertEqual(summary.seen(sha, Api([], runs=runs)), "", "dry and cancelled runs do not")

    def test_nothing_is_written_off_main_or_in_a_dry_run(self):
        jobs = [job("shards / ignored-tests-shards (horizon-world)", "failure", ["Run"])] + GREEN_JOBS[2:]
        for ref, dry_run in (("refs/heads/hotspots-daily", "false"), ("refs/heads/main", "true")):
            with self.subTest(ref=ref, dry_run=dry_run):
                api = Api(jobs)
                text = self.run_summary(api, [shard_report([failure("a::b")])], ref=ref, dry_run=dry_run,
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


if __name__ == "__main__":
    unittest.main()
