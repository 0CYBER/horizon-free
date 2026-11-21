#!/usr/bin/env python3
"""The daily run's summary (.github/workflows/daily.yml): one issue per failing test.

Reads this run's jobs (names, conclusions and step names from the Actions API, never a log) and the legs' verdict
files (tools/ci/ignored_tests.py --report: test identity, kind, panic location, policy file), then:
- a failing test: files `Daily: <crate> <test>` (label `bug`) with its panic location and this run as the first
  that saw it, or comments on the open one; when its newest issue was closed by a pull request or commit that is not in
  this run's commit (a fix merged after the commit under test), it comments on that closed issue and leaves it closed
  instead of filing a new one;
- a leg that failed without a test name (build error, timeout, killed, cancelled, skipped, or a leg with no
  sanitised verdict): one issue per leg, `Daily: <leg> did not complete`, naming the failed steps and, for a shard,
  why each run stopped as its verdict file says (signal or exit status, memory peak and limit, wall time, the tests
  still running past 60 s; stopped_text);
- a test (or leg) green again: closes its open issue with a comment.
A job it does not know, or an expected leg with no job, is a red leg ("did not report"), never green; so is a
shard of the matrix (tools/ci/ignored_shards.py, which builds it) with no job of its name. Each comment
names this run's URL, and none is posted twice for one run (a rerun of failed jobs, or a retry after a partial
write, completes what is missing). `daily_summary.py --seen SHA` prints a completed real daily run of SHA, if any.
Only a run on refs/heads/main with DRY_RUN=false writes; otherwise it prints what it would do to the job summary.
Lookups come first and fail the job on any API error (a rate limit included), so a failed lookup never files
duplicates.

Environment: GITHUB_REPOSITORY, GITHUB_RUN_ID, GITHUB_REF, GITHUB_SERVER_URL, GH_TOKEN, COMMIT, DRY_RUN,
RESULTS (toJSON(needs)), REPORTS (the downloaded daily-report-* artifacts), GITHUB_STEP_SUMMARY.
"""

import datetime as dt
import json
import os
import re
import subprocess
import sys
import urllib.error
import urllib.request
from pathlib import Path

sys.path.append(str(Path(__file__).resolve().parents[2]))
from tools.ci import ignored_shards  # noqa: E402

LABEL = "bug"
PREFIX = "Daily: "
UNFINISHED = " did not complete"
DISCLOSURE = "> 🤖 Filed by the daily run."
# daily.yml's leg jobs (each a called workflow), and the called workflows' own routing jobs, which test nothing.
LEGS = ("shards", "wasm-mods", "fn-replace-check", "linkage", "windows-runtime-leases")
HELPERS = ("plan", "route", "fresh", "gate")
# Jobs of a called workflow that the daily run does not use: launcher-windows-check's push and pull-request tests,
# and the wasm fixture, which needs the vanilla data and runs by dispatch only (wasm-mods.yml).
NOT_DAILY = ("windows-runtime-leases / test", "wasm-mods / fixture")
MAX_PAGES = 100
# How far back a closed issue's fix is looked for: a fix that merged after the commit under test is hours old.
FIX_LOOKBACK = dt.timedelta(days=14)
# What closed an issue: a pull request (its merge commit) or a commit that named it.
CLOSER = """query($owner: String!, $name: String!, $number: Int!) {
  repository(owner: $owner, name: $name) { issue(number: $number) {
    timelineItems(itemTypes: [CLOSED_EVENT], last: 1) { nodes { ... on ClosedEvent {
      closer { ... on PullRequest { mergeCommit { oid } } ... on Commit { oid } } } } } } }
}"""


def send(url, method, body, what):
    """One GitHub HTTP call; any HTTP error (a rate limit included) ends the job."""
    request = urllib.request.Request(
        url, method=method, data=json.dumps(body).encode() if body is not None else None,
        headers={"Authorization": f"Bearer {os.environ['GH_TOKEN']}", "Accept": "application/vnd.github+json",
                 "X-GitHub-Api-Version": "2022-11-28"})
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            return json.load(response) if response.status != 204 else None
    except urllib.error.HTTPError as error:
        remaining = error.headers.get("x-ratelimit-remaining")
        sys.exit(f"::error::GitHub refused {what}: HTTP {error.code}"
                 + (" (rate limit exhausted)" if remaining == "0" else "")
                 + "; nothing more is written, so no duplicate issue can follow")


def api(method, path, body=None):
    """One GitHub REST call on this repository."""
    return send(f"https://api.github.com/repos/{os.environ['GITHUB_REPOSITORY']}/{path}", method, body,
                f"{method} {path}")


def graphql(query, variables):
    """One GitHub GraphQL query; an error in its answer ends the job like an HTTP error."""
    answer = send("https://api.github.com/graphql", "POST", {"query": query, "variables": variables}, "a GraphQL query")
    if answer.get("errors"):
        sys.exit(f"::error::GitHub refused a GraphQL query: {answer['errors'][0].get('message')}; "
                 "nothing more is written, so no duplicate issue can follow")
    return answer["data"]


def pages(call, path, key=None):
    """Every item of a paginated listing; refuses to act on a listing it could not read whole."""
    items = []
    for page in range(1, MAX_PAGES + 1):
        data = call("GET", f"{path}{'&' if '?' in path else '?'}per_page=100&page={page}")
        batch = data[key] if key else data
        items.extend(batch)
        if len(batch) < 100:
            return items
    sys.exit(f"::error::{path} has more than {MAX_PAGES * 100} items; refusing to act on a partial listing")


def leg_jobs(jobs):
    """This run's leg jobs (`<leg> / <job>`), without the called workflows' routing jobs."""
    return [job for job in jobs
            if job["name"].split(" / ", 1)[0] in LEGS and " / " in job["name"]
            and job["name"].split(" / ", 1)[1] not in HELPERS and job["name"] not in NOT_DAILY]


def expected_shards():
    """The `shards` leg's matrix jobs the daily run must have, from the matrix's own source
    (tools/ci/ignored_shards.py --matrix --all): every package shard and every daily-rows leg."""
    return [f"shards / ignored-tests-shards ({shard})"
            for shard in [*ignored_shards.expand(ignored_shards.SHARDS), *ignored_shards.DAILY_ROWS_LEGS.values()]]


def job_green(job):
    return job["conclusion"] == "success"


def failed_helpers(jobs):
    """leg -> its routing or helper jobs that did not succeed (whose failure skips the leg's real jobs)."""
    failed = {}
    for job in jobs:
        leg, _, rest = job["name"].partition(" / ")
        if leg in LEGS and rest in HELPERS and job["conclusion"] not in ("success", "skipped"):
            failed.setdefault(leg, []).append(job["name"])
    return failed


def failed_steps(job):
    return [step["name"] for step in job.get("steps") or []
            if step.get("conclusion") in ("failure", "cancelled", "timed_out")]


def report_for(job, reports):
    """The verdict file a leg job wrote: a shard's names its shard."""
    name = job["name"]
    for report in reports:
        if any(name.endswith(f"({shard})") for shard in report.get("shards", ())):
            return report
    return None


def known(name):
    """daily.yml's own jobs and its legs' jobs; any other job is one the summary cannot judge."""
    return (name in ("resolve", "schedule-dispatch") or name.startswith("summary ")
            or name.split(" / ", 1)[0] in LEGS and " / " in name)


# A run's name and a test's name as ignored_tests.py writes them; anything else in a report is not shown.
SHARD_NAME = re.compile(r"[\w#:.-]{1,200}")
TEST_NAME = re.compile(r"(?=.{1,300}$)(?:r#)?[^\W\d]\w*(?:::(?:r#)?[^\W\d]\w*)*")
STILL_RUNNING_SHOWN = 5


def number(value):
    return value if isinstance(value, int) and not isinstance(value, bool) and value >= 0 else None


def gib(size):
    return f"{size / 1024 ** 3:.1f} GiB"


def stopped_text(entry):
    """Why one run of a shard did not complete (ignored_tests.py's report: names and numbers only), as one sentence:
    "`horizon-client#1of2` killed by signal 9 (SIGKILL), out of memory (1 kill); memory peak 32.0 GiB of 32.0 GiB;
    after 70 min; still running: `a::b`". Unknown fields are left out, never guessed."""
    shard = entry.get("shard")
    name = f"`{shard}`" if isinstance(shard, str) and SHARD_NAME.fullmatch(shard) else "a run"
    if entry.get("in_progress") is True:
        return (f"{name} was still running when the leg ended (its time limit, a cancel, or a lost runner); which "
                "tests were running and the process's end are not known")
    parts = []
    ended = entry.get("ended_by")
    if isinstance(ended, str) and re.fullmatch(r"(?:signal|exit status) \d{1,5}(?: \(SIG[A-Z0-9]{1,10}\))?", ended):
        parts.append(f"{name} killed by {ended}" if ended.startswith("signal") else f"{name} ended with {ended}")
    else:
        reason = entry.get("reason")
        parts.append(f"{name}: {reason}" if isinstance(reason, str) and re.fullmatch(r"[a-z ()]{1,120}", reason)
                     else f"{name} did not complete")
        if (code := number(entry.get("cargo_exit"))) is not None:
            parts[-1] += f" (cargo exit {code})"
    if kills := number(entry.get("oom_kills")):
        parts[-1] += f", out of memory ({kills} kill{'s' if kills > 1 else ''} by the kernel)"
    peak, limit = number(entry.get("memory_peak_bytes")), number(entry.get("memory_max_bytes"))
    if peak is not None:
        parts.append(f"job memory peak {gib(peak)}" + (f" of {gib(limit)}" if limit is not None else ""))
    if (seconds := number(entry.get("wall_seconds"))) is not None:
        parts.append(f"after {seconds // 60} min" if seconds >= 120 else f"after {seconds} s")
    still = [test for test in entry.get("still_running") or () if isinstance(test, str) and TEST_NAME.fullmatch(test)]
    if still:
        more = len(still) - STILL_RUNNING_SHOWN + (number(entry.get("still_running_more")) or 0)
        parts.append("still running past 60 s: " + ", ".join(f"`{test}`" for test in still[:STILL_RUNNING_SHOWN])
                     + (f" and {more} more" if more > 0 else ""))
    if never := number(entry.get("never_reported")):
        parts.append(f"{never} listed test{'s' if never > 1 else ''} never reported")
    return "; ".join(parts)


def named(problem):
    return not problem["test"].startswith("identity withheld")


def verdicts(jobs, reports, results):
    """(failing tests, unfinished legs, passing tests, green legs) of this run.

    A failing test is a dict of package, test, kind, panic, policy and leg; an unfinished leg is (leg, conclusion,
    failed steps, reason). Green means every leg job and every called workflow concluded success."""
    tests, unfinished, passed, green_legs = [], [], set(), set()
    helpers = failed_helpers(jobs)
    for job in leg_jobs(jobs):
        report = report_for(job, reports)
        if report:
            passed |= {(item["package"], item["test"]) for item in report["passed"]}
        if job_green(job):
            green_legs.add(job["name"])
            continue
        if job["conclusion"] == "skipped" and job["name"].split(" / ", 1)[0] in helpers:
            continue  # its leg's failed helper is the one item for it, below
        steps = failed_steps(job)
        problems = [dict(item, leg=job["name"]) for item in (report or {}).get("problems", ())]
        by_name = [item for item in problems if named(item)]
        if job["name"].startswith("shards / "):
            tests += by_name
            if report is None:
                unfinished.append((job["name"], job["conclusion"], steps, "no verdict file: the leg ended before its tests reported"))
            elif report["incomplete"] or len(by_name) < len(problems) or not problems:
                reason = ("a test run did not complete: " + ". ".join(
                    stopped_text(entry if isinstance(entry, dict) else {}) for entry in report["incomplete"])
                          if report["incomplete"] else
                          "failed tests whose names are withheld" if problems else "failed outside its tests")
                unfinished.append((job["name"], job["conclusion"], steps, reason))
        else:
            unfinished.append((job["name"], job["conclusion"], steps, "failed at the steps named"))
    for job in jobs:
        if not known(job["name"]):
            unfinished.append((job["name"], job["conclusion"], [], "did not report: a job the summary does not know"))
    names = {job["name"] for job in jobs}
    if "shards" not in helpers:
        for expected in expected_shards():
            if expected not in names:
                unfinished.append((expected, "missing", [], "did not report: the daily matrix has this shard, but "
                                   "no job of that name ran"))
    seen = {job["name"].split(" / ", 1)[0] for job in leg_jobs(jobs)}
    for leg in LEGS:
        result = results.get(leg, {}).get("result", "missing")
        accounted = any(item["leg"].split(" / ", 1)[0] == leg for item in tests) or any(
            entry[0].split(" / ", 1)[0] == leg for entry in unfinished)
        if leg in helpers:
            unfinished.append((leg, result, [], "did not complete: " + ", ".join(f"`{name}`" for name in helpers[leg])
                               + " failed, so its jobs did not run"))
        elif leg not in seen:
            unfinished.append((leg, result, [], "did not report: no job of this leg ran, or its jobs are named differently"))
        elif result != "success" and not accounted:
            unfinished.append((leg, result, [], "did not complete, and no job or test of it says why"))
        elif result == "success" and not accounted:
            green_legs.add(leg)  # a whole leg that reported and passed closes its leg-level issue
    failing = {(item["package"], item["test"]) for item in tests}
    return tests, unfinished, passed - failing, green_legs


def test_title(item):
    return f"{PREFIX}{item['package']} {item['test']}"


def leg_title(leg):
    return f"{PREFIX}{leg}{UNFINISHED}"


def detail(item):
    where = f"`{item['panic']}`" if item["panic"] else "not reported"
    kinds = {
        "failed": f"fails in the daily run. Panic location: {where}.",
        "gpu": f"fails with a Vulkan driver present. Panic location: {where}.",
        "passes-now": f"passes now but is listed as a known failure in `{item['policy']}`: take it off that list.",
        "no-longer-runs": f"is listed in `{item['policy']}` but no longer runs: remove the row or restore the test.",
    }
    return f"`{item['test']}` ({item['package']}, leg `{item['leg']}`) {kinds[item['kind']]}"


def leg_detail(leg, conclusion, steps, reason):
    named_steps = ", ".join(f"`{step}`" for step in steps) or "none recorded"
    return f"Leg `{leg}` ended `{conclusion}`: {reason}. Failed steps: {named_steps}."


def fixes_after(commit, titles, call, query, contains=None):
    """title -> (number, fix) for each red title with no open issue whose newest closed issue was closed by a pull
    request or commit `fix` that `commit` does not contain: the fix merged after the commit this run tested.
    `contains(fix, commit)` answers True/False, or None when git cannot say; that is not evidence about the fix."""
    contains = contains or contains_fix
    if not titles:
        return {}
    since = (dt.datetime.now(dt.timezone.utc) - FIX_LOOKBACK).date().isoformat()
    newest = {}
    for issue in pages(call, f"issues?state=closed&labels={LABEL}&since={since}"):
        if "pull_request" not in issue and issue["title"] in titles:
            newest[issue["title"]] = max(newest.get(issue["title"], 0), issue["number"])
    owner, name = os.environ["GITHUB_REPOSITORY"].split("/")
    fixes = {}
    for title, number in sorted(newest.items()):
        nodes = query(CLOSER, {"owner": owner, "name": name, "number": number})["repository"]["issue"][
            "timelineItems"]["nodes"]
        closer = (nodes[-1].get("closer") if nodes else None) or {}
        fix = (closer.get("mergeCommit") or {}).get("oid") or closer.get("oid")
        if not fix:
            continue
        answer = contains(fix, commit)
        if answer is None:
            sys.exit(f"::error::git could not say whether `{commit[:9]}` contains the fix `{fix[:9]}`; nothing more "
                     "is written, so a regression issue is never skipped on a git error")
        # A commit the checkout does not hold is not in `commit` either: the checkout holds all of its history.
        if not answer:
            fixes[title] = (number, fix)
    return fixes


def plan(commit, run_url, tests, unfinished, passed, green_legs, open_issues, fixes=None):
    """Every write this run makes, in order: ("create", title, body) / ("comment", number, title, body)
    / ("close", number, title, body). `fixes` (fixes_after) holds the closed issues whose fix this commit predates."""
    fixes = fixes or {}
    actions = []
    reds = [(test_title(item), detail(item)) for item in tests]
    reds += [(leg_title(leg), leg_detail(leg, conclusion, steps, reason)) for leg, conclusion, steps, reason in unfinished]
    filed = set()
    for title, text in reds:
        if title in filed:
            continue
        filed.add(title)
        if title in open_issues:
            actions.append(("comment", open_issues[title], title,
                            f"🤖 Agent: {red_marks(run_url)[1]} on `{commit[:9]}`.\n\n{text}\n"))
        elif title in fixes:
            number, fix = fixes[title]
            actions.append(("comment", number, title,
                            f"🤖 Agent: {red_marks(run_url)[1]} on `{commit[:9]}`, which predates the fix `{fix[:9]}`"
                            f" that closed this issue; it stays closed.\n\n{text}\n"))
        else:
            actions.append(("create", title,
                            f"{DISCLOSURE}\n\n{text}\n\n{red_marks(run_url)[0]} on `{commit[:9]}`.\n\n"
                            f"This issue gets a comment from each later daily run that still sees it and is "
                            "closed by the first that does not.\n"))
    for title, number in sorted(open_issues.items()):
        if title in filed:
            continue
        rest = title[len(PREFIX):]
        if rest.endswith(UNFINISHED):
            back = rest[:-len(UNFINISHED)] in green_legs
        else:
            package, _, test = rest.partition(" ")
            back = (package, test) in passed
        if back:
            actions.append(("close", number, title,
                            f"🤖 Agent: {green_mark(run_url)} on `{commit[:9]}`; closing.\n"))
    return actions


def render(actions, writes):
    lead = "## Daily summary" + ("" if writes else " (dry run: nothing written)")
    lines = [lead, ""]
    if not actions:
        lines.append("Nothing to do: no leg is red and no open daily issue went green.")
    for action in actions:
        verb = {"create": "File", "comment": "Comment on", "close": "Close"}[action[0]]
        if action[0] == "create":
            lines += [f"- {verb} **{action[1]}**", "", "  " + action[2].strip().replace("\n", "\n  "), ""]
        else:
            lines += [f"- {verb} #{action[1]} **{action[2]}**", "", "  " + action[3].strip().replace("\n", "\n  "), ""]
    return "\n".join(lines) + "\n"


def red_marks(run_url):
    """What a red report of this run says on an issue: its opening body, or its comment."""
    return (f"First daily run that saw it: {run_url}", f"Still red in the daily run {run_url}")


def green_mark(run_url):
    return f"Green again in the daily run {run_url}"


def already_said(number, marks, call):
    """Whether issue `number` already carries one of `marks` (its body, or one of its comments): one comment per
    transition and run, so a rerun that turns a test green still posts its close comment."""
    texts = [call("GET", f"issues/{number}")["body"] or ""]
    texts += [comment["body"] or "" for comment in pages(call, f"issues/{number}/comments")]
    return any(mark in text for mark in marks for text in texts)


def apply(actions, call, run_url):
    for action in actions:
        if action[0] == "create":
            call("POST", "issues", {"title": action[1], "body": action[2], "labels": [LABEL]})
        elif action[0] == "comment":
            if not already_said(action[1], red_marks(run_url), call):
                call("POST", f"issues/{action[1]}/comments", {"body": action[3]})
        else:
            if not already_said(action[1], (green_mark(run_url),), call):
                call("POST", f"issues/{action[1]}/comments", {"body": action[3]})
            call("PATCH", f"issues/{action[1]}", {"state": "closed", "state_reason": "completed"})


def contains_fix(fix, commit):
    """True when `commit` already holds `fix`; False when it does not; None when git could not answer.
    `git merge-base --is-ancestor` exits 0 for an ancestor, 1 for a non-ancestor and 128+ for an error."""
    result = subprocess.run(["git", "merge-base", "--is-ancestor", fix, commit], capture_output=True, text=True)
    if result.returncode == 0:
        return True
    return False if result.returncode == 1 else None


def main(call=api, run_git=contains_fix, query=graphql):
    env = os.environ
    commit = env["COMMIT"]
    # Only main writes, whatever the input says: a branch's run is always a dry run.
    writes = env.get("GITHUB_REF") == "refs/heads/main" and env.get("DRY_RUN") == "false"
    run_url = f"{env.get('GITHUB_SERVER_URL', 'https://github.com')}/{env['GITHUB_REPOSITORY']}/actions/runs/{env['GITHUB_RUN_ID']}"
    reports = [json.loads(path.read_text(encoding="utf-8")) for path in sorted(Path(env["REPORTS"]).glob("*/*.json"))] \
        if Path(env.get("REPORTS", "")).is_dir() else []
    jobs = pages(call, f"actions/runs/{env['GITHUB_RUN_ID']}/jobs?filter=latest", "jobs")
    tests, unfinished, passed, green_legs = verdicts(jobs, reports, json.loads(env["RESULTS"]))
    open_issues = {issue["title"]: issue["number"]
                   for issue in pages(call, f"issues?state=open&labels={LABEL}")
                   if "pull_request" not in issue and issue["title"].startswith(PREFIX)}
    reds = {test_title(item) for item in tests} | {leg_title(leg) for leg, *_ in unfinished}
    fixes = fixes_after(commit, reds - open_issues.keys(), call, query, run_git)
    actions = plan(commit, run_url, tests, unfinished, passed, green_legs, open_issues, fixes)
    text = render(actions, writes)
    print(text)
    if env.get("GITHUB_STEP_SUMMARY"):
        with open(env["GITHUB_STEP_SUMMARY"], "a", encoding="utf-8") as summary:
            summary.write(text)
    if writes:
        apply(actions, call, run_url)
    return 0


def seen(sha, call=api):
    """The URL of a completed real daily run of sha (its summary job, named `summary <sha>`, finished), or ""."""
    current = int(os.environ["GITHUB_RUN_ID"])
    # The schedule asks about main's head, so a run of it is recent: look back two weeks, not at all history.
    since = (dt.datetime.now(dt.timezone.utc) - dt.timedelta(days=14)).date().isoformat()
    for run in pages(call, f"actions/workflows/daily.yml/runs?status=completed&created=%3E%3D{since}", "workflow_runs"):
        if run["id"] == current or run["conclusion"] not in ("success", "failure"):
            continue
        if any(job["name"] == f"summary {sha}" and job["conclusion"] in ("success", "failure")
               for job in pages(call, f"actions/runs/{run['id']}/jobs?filter=latest", "jobs")):
            return run["html_url"]
    return ""


if __name__ == "__main__":
    if sys.argv[1:2] == ["--seen"]:
        print(seen(sys.argv[2]))
        sys.exit(0)
    sys.exit(main())
