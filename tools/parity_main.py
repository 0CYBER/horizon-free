#!/usr/bin/env python3
"""parity-main (.github/workflows/parity-main.yml): the vanilla harnesses run on main, a batch of merges at a time.

    python3 tools/parity_main.py base               # the commit this run checks from: main's last green parity-main
    python3 tools/parity_main.py verdict BASE       # after a failure: the run's verdict, the failed harnesses run again
    python3 tools/parity_main.py bisect BASE        # the same, then the first merge since BASE that broke a harness
    python3 tools/parity_main.py history            # a run whose harnesses passed: kept in the flake history

`base` prints the newest commit on main whose parity-main run passed and that HEAD contains, else HEAD's first parent,
so a run tests every merge since the last green one (`tools/parity.py --changed <base>`), however many landed while
the previous run went on. `verdict` reads the harnesses that failed (.local/parity-runner/last-run.json, only this
run's when PARITY_RUN_ID is set) and runs them again at HEAD: it exits 0 when they all pass there (a flake, counted as a
pass), else 1, and writes verdict-<PARITY_RUN_ID>.json (each failed harness's tries, and the repeat flakers: those
not on known_flaky that needed that rerun to pass in 3 of the last 10 runs). The gate runs it; `bisect` does the same and, where some
fail again, checks out the first-parent merges between BASE and HEAD by halves and runs only those harnesses, then
names, for each of them that fails again at HEAD and passes at BASE, the first merge it fails on (written to $GITHUB_STEP_SUMMARY and, with --comment, on its pull request). HEAD is checked out again at the end.

The search has --budget minutes: the job's limit otherwise cuts it off with nothing said (seven runs between
2026-11-09 and 11-11, searching up to 1,500 merges). When they are spent it says between which merges each harness
broke and stops. A merge is named only when its harness fails there again and passes again at the commit before it: the search runs
each merge once, so a harness that fails some of the time would otherwise end on any merge. What it learnt stays in .local/parity-runner/bisect.json while the base is the same, so the next
run goes on from there, and a merge it already named is not commented on again.
"""

import argparse
from collections import Counter
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
REPO = os.environ.get("GITHUB_REPOSITORY", "horizon-rust/horizon")
LAST_RUN = ROOT / ".local" / "parity-runner" / "last-run.json"
# {"since": the base, "results": {commit: {harness: it failed there}}, "named": {harness: the merge commented on}}
BISECT = ROOT / ".local" / "parity-runner" / "bisect.json"
# tools/parity.py's record of each harness's last run (whether it was retried); read for the verdict report.
STATE = ROOT / ".local" / "parity-runner" / "state.json"
# What `verdict` found, as verdict-<PARITY_RUN_ID>.json where that is set (verdict_path); the workflow uploads it.
VERDICT = ROOT / ".local" / "parity-runner" / "verdict.json"
# The last HISTORY_RUNS gate runs, oldest first: [{"run", "head", "rerun_passed": [harness]}]. Kept on the runner
# beside state.json, so no API call; written whole and renamed, so a cancelled run leaves the old file, never half one.
HISTORY = ROOT / ".local" / "parity-runner" / "verdict-history.json"
HISTORY_RUNS, REPEATS = 10, 3


def git(*arguments, check=True):
    result = subprocess.run(["git", *arguments], cwd=ROOT, capture_output=True, text=True)
    if check and result.returncode:
        sys.exit(f"git {' '.join(arguments)}: {result.stderr.strip()}")
    return result


def green_runs():
    """Head commits of main's passing parity-main runs, newest first."""
    result = subprocess.run(["gh", "run", "list", "--repo", REPO, "--workflow", "parity-main", "--branch", "main",
                             "--status", "success", "--limit", "30", "--json", "headSha"], capture_output=True, text=True)
    if result.returncode:
        print(f"note: no parity-main history ({result.stderr.strip()})", file=sys.stderr)
        return []
    return [run["headSha"] for run in json.loads(result.stdout or "[]")]


def base(head="HEAD", runs=None):
    for sha in green_runs() if runs is None else runs:
        if git("merge-base", "--is-ancestor", sha, head, check=False).returncode == 0:
            return git("rev-parse", sha).stdout.strip()
    return git("rev-parse", f"{head}^1").stdout.strip()


def first_bad(commits, fails):
    """The first of commits (oldest first, the last known to fail; the one before them known to pass) where fails()."""
    good, bad = -1, len(commits) - 1
    while bad - good > 1:
        middle = (good + bad) // 2
        if fails(commits[middle]):
            bad = middle
        else:
            good = middle
    return commits[bad]


def harness_exists(name):
    return (ROOT / "tools" / f"test_{name.split('#')[0]}_parity.py").exists()


def run_harnesses(names, timeout):
    """The harnesses that fail at the checked-out commit (those it doesn't have yet can't), as parity.py records them."""
    present = [name for name in names if harness_exists(name)]
    if not present:
        return []
    if subprocess.run([sys.executable, "tools/parity.py", *present, "--timeout", str(timeout)], cwd=ROOT).returncode == 0:
        return []
    try:
        failed = json.loads(LAST_RUN.read_text(encoding="utf-8")).get("failed") or []
    except (OSError, ValueError):
        failed = []
    # A run that failed before recording its harnesses (a probe that doesn't build) fails them all.
    return [name for name in present if name in failed] or present


class OutOfTime(Exception):
    pass


def earlier_search(since):
    """What earlier runs' searches from this base learnt, or a new record: another base has other merges to search."""
    try:
        state = json.loads(BISECT.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        state = {}
    if state.get("since") != since:
        state = {"since": since, "results": {}, "named": {}}
    return state


def report(line):
    print(line)
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a", encoding="utf-8") as summary:
            summary.write(f"### {line}\n")


def hosts_entries(key):
    """tools/parity_hosts.json's known_failing or known_flaky ({harness: reason}), or none where it can't be read."""
    try:
        return json.loads((ROOT / "tools" / "parity_hosts.json").read_text(encoding="utf-8")).get(key, {})
    except (OSError, ValueError):
        return {}


def judge_at_head(since, timeout):
    """All `verdict` does, and what `bisect` does before its search: read the harnesses the run's batch failed and run
    them again at HEAD, recording that in bisect.json and checking HEAD out again. Returns (code, failed, at_head, head,
    commits, the bisect.json state): code 1 where there is nothing to judge (no rerun), 0 where every failed harness
    passes when run again (a flake, counted as a pass), None where those of at_head fail again."""
    try:
        recorded = json.loads(LAST_RUN.read_text(encoding="utf-8")) if LAST_RUN.exists() else {}
        if not isinstance(recorded, dict):
            raise ValueError("not a JSON object")
    except (OSError, ValueError) as error:
        print(f"{LAST_RUN.name} can't be read ({error}): the run fails.")
        return 1, [], [], None, [], None
    # tools/parity.py records its PARITY_RUN_ID with the list. One that stopped before recording leaves an earlier
    # run's list (the run before, or a bisect's last probe), whose harnesses may pass here: that passed a batch
    # nothing judged. Without the variable (a run by hand) any list is taken, as before.
    run_id = os.environ.get("PARITY_RUN_ID")
    if run_id and recorded.get("run") != run_id:
        report(f"parity-main: {LAST_RUN.name} is from run {recorded.get('run')}, not this one ({run_id}): "
               "tools/parity.py stopped before recording its failed harnesses, so the run fails.")
        return 1, [], [], None, [], None
    failed = recorded.get("failed") or []
    # tools/parity.py doesn't fail a run for a known failing harness (tools/parity_hosts.json), so a probe running one
    # would pass where it fails, and a merge that broke another harness could be blamed for it too.
    known = hosts_entries("known_failing")
    failed = [name for name in failed if name not in known and name.split("#")[0] not in known]
    if not failed:
        print("No failed harness recorded, known failing ones aside: nothing to bisect.")
        return 1, [], [], None, [], None
    head = git("rev-parse", "HEAD").stdout.strip()
    commits = git("rev-list", "--first-parent", "--reverse", f"{since}..{head}").stdout.split()
    if not commits:
        print("No merge since the base.")
        return 1, failed, [], head, commits, None
    state = earlier_search(since)
    # HEAD is run again whatever an earlier run saw there: it is the run's verdict.
    state["results"].pop(head, None)
    try:
        print(f"--- {head[:9]} {git('log', '-1', '--format=%s', head).stdout.strip()}", flush=True)
        git("checkout", "-q", "--detach", head)
        at_head = run_harnesses(failed, timeout)
        state["results"][head] = {name: name in at_head for name in failed}
        # Written on every path, as `bisect` always did: a run whose record can't be written is red, for both commands.
        BISECT.parent.mkdir(parents=True, exist_ok=True)
        BISECT.write_text(json.dumps(state), encoding="utf-8")
        flakes = [name for name in failed if name not in at_head]
        if flakes:
            report(f"parity-main: {', '.join(flakes)} pass when run again at {head[:9]}: a flake, counted as a pass.")
    finally:
        # HEAD checked out again however the rerun ended, as `bisect` always did (its search does it again at its end).
        git("checkout", "-q", "--detach", head)
    return (None if at_head else 0), failed, at_head, head, commits, state


def harness_records():
    try:
        return json.loads(STATE.read_text(encoding="utf-8")).get("harnesses", {})
    except (OSError, ValueError, AttributeError):
        return {}


def tries(record, step):
    """A harness's runs in one tools/parity.py run, from its state.json record: a failed one before its retry."""
    retried = record.get("retried")
    last = {"try": f"{step}, retried ({retried})" if retried else step, "result": record.get("status", "fail")}
    return [{"try": step, "result": "fail"}, last] if retried else [last]


def write_whole(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(path.name + ".tmp")
    temporary.write_text(json.dumps(value, indent=1), encoding="utf-8")
    temporary.replace(path)


def flake_history(entry):
    """The last HISTORY_RUNS runs, this one (entry) last; saved for the next runs when it has a run id."""
    try:
        runs = json.loads(HISTORY.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        runs = []
    runs = [run for run in runs if isinstance(run, dict) and run.get("run") != entry["run"]] \
        if isinstance(runs, list) else []
    runs = (runs + [entry])[-HISTORY_RUNS:]
    if entry["run"]:
        write_whole(HISTORY, runs)
    return runs


def repeat_flakers(runs):
    """[(harness, how many of runs it passed in only when run again)] for those that did in REPEATS or more and are
    not on known_flaky (tools/parity.py retries those itself, and their entry already says why)."""
    flaky = hosts_entries("known_flaky")
    counts = Counter(name for run in runs if isinstance(run.get("rerun_passed"), list)
                     for name in set(run["rerun_passed"]) if isinstance(name, str))
    return sorted((name, count) for name, count in counts.items()
                  if count >= REPEATS and name not in flaky and name.split("#")[0] not in flaky)


def verdict_path():
    """This attempt's report (verdict-<run id>-<attempt>.json), so the workflow never uploads an earlier one's."""
    run_id = os.environ.get("PARITY_RUN_ID")
    return VERDICT.with_name(f"verdict-{run_id}.json") if run_id else VERDICT


def verdict(since, timeout):
    """The gate run's verdict: judge_at_head's exit code, and 1 where `bisect` would go on to search. Writes the report
    (verdict_path) and the flake history; neither, nor their failing, changes the exit code."""
    path = verdict_path()
    try:
        path.unlink(missing_ok=True)
    except OSError as error:
        print(f"note: {path.name} left from before ({error!r})", file=sys.stderr)
    before = harness_records()
    code, failed, at_head, head, _, _ = judge_at_head(since, timeout)
    reran = code != 1
    code = 1 if code is None else code
    # Each part is on its own: a failing history never keeps the report from being written, nor either the exit code.
    harnesses, flakers = {}, []
    try:
        after = harness_records()
        for name in failed:
            harnesses[name] = tries(before.get(name) or {"status": "fail"}, "batch")
            if reran:
                # A rerun tools/parity.py didn't record (it stopped) leaves the batch's record: only its result is known.
                rerun = after.get(name) if after.get(name) not in (None, before.get(name)) else {}
                harnesses[name] += tries(dict(rerun, status="fail" if name in at_head else "pass"), "rerun at HEAD")
        head = head or git("rev-parse", "HEAD", check=False).stdout.strip()
    except Exception as error:  # noqa: BLE001 - a record of the run is never worth changing its verdict
        print(f"note: the tries are incomplete ({error!r})", file=sys.stderr)
    try:
        passed_again = [name for name in failed if name not in at_head] if reran else []
        flakers = repeat_flakers(flake_history({"run": os.environ.get("PARITY_RUN_ID"), "head": head,
                                                "rerun_passed": passed_again}))
        for name, count in flakers:
            report(f"parity-main: {name} passed only when run again in {count} of the last {HISTORY_RUNS} runs and "
                   "is not on known_flaky (tools/parity_hosts.json): a repeat flaker. It doesn't fail the run.")
    except Exception as error:  # noqa: BLE001
        print(f"note: no flake history ({error!r})", file=sys.stderr)
    try:
        write_whole(path, {"head": head, "base": since, "run_id": os.environ.get("GITHUB_RUN_ID"),
                           "run_attempt": os.environ.get("GITHUB_RUN_ATTEMPT"), "exit_code": code,
                           "harnesses": harnesses, "repeat_flakers": [name for name, _ in flakers]})
    except Exception as error:  # noqa: BLE001
        print(f"note: {path.name} not written ({error!r})", file=sys.stderr)
    return code


def history():
    """A run whose batch passed: kept in the flake history as one no harness needed the rerun in, so repeat_flakers
    counts every run, not only the failed ones."""
    if not os.environ.get("PARITY_RUN_ID"):
        sys.exit("history: PARITY_RUN_ID is not set")
    flake_history({"run": os.environ["PARITY_RUN_ID"], "head": git("rev-parse", "HEAD").stdout.strip(),
                   "rerun_passed": []})
    return 0


def bisect(since, timeout, comment, budget=90):
    code, failed, at_head, head, commits, state = judge_at_head(since, timeout)
    if code is not None:
        return code
    results = state["results"]
    deadline = None  # set once the base ran: the budget is for the search between it and HEAD

    def fails(commit, names, again=False):
        """Those of names that fail at commit: from an earlier run of them there, else (or with again) by running
        them now."""
        known = results.setdefault(commit, {})
        missing = [name for name in names if again or name not in known]
        if missing:
            if deadline is not None and time.monotonic() >= deadline:
                raise OutOfTime
            print(f"--- {commit[:9]} {git('log', '-1', '--format=%s', commit).stdout.strip()}", flush=True)
            git("checkout", "-q", "--detach", commit)
            failed_here = run_harnesses(missing, timeout)
            known.update({name: name in failed_here for name in missing})
            BISECT.parent.mkdir(parents=True, exist_ok=True)
            BISECT.write_text(json.dumps(state), encoding="utf-8")
        return [name for name in names if known[name]]

    # Each harness is judged on its own. One that passes when run again at HEAD is a flake (counted as a pass,
    # judge_at_head); one that fails at the base too is the runner's or a flake (no merge to blame: the search would
    # end on the oldest merge); the others fail at HEAD and pass at the base, so a merge broke them and the halving
    # search names it. Harnesses that break at different merges are searched again after the first one. The exit code
    # is the run's verdict, as `verdict`'s: 0 only when every failed harness passes when run again at HEAD (the commit
    # is good, so releases may follow it), 1 otherwise.
    blames = []
    remaining = []
    try:
        at_base = fails(since, at_head)
        deadline = time.monotonic() + budget * 60
        if at_base:
            report(f"parity-main: {', '.join(at_base)} fail at the base {since[:9]} too: the runner or a flake: "
                   "no merge is named.")
        remaining = [name for name in at_head if name not in at_base]

        def failing(commit):
            return fails(commit, remaining)

        def known(commit):
            """Whether a remaining harness fails at commit, or None where not all of them ran yet."""
            ran = results.get(commit, {})
            return any(ran[name] for name in remaining) if all(name in ran for name in remaining) else None

        low = 0
        while remaining:
            # Between the last merge known to pass and the first known to fail: every merge in a first search, fewer
            # once an earlier run, or this run for another harness, tried merges in between.
            high = next(index for index in range(low, len(commits)) if known(commits[index]))
            low = max([index + 1 for index in range(low, high) if known(commits[index]) is False], default=low)
            span = commits[low:high + 1]
            culprit = first_bad(span, failing) if len(span) > 1 else span[0]
            hit = failing(culprit)
            # The search took one run's word at each merge, so a harness that fails only some of the time ends on
            # whichever merge follows its last lucky pass (2026-11-15: #16690 named for pearl_wind_charge, which fails
            # one run in three before it). A merge is named for a harness that fails there once more and passes once
            # more at the commit before; the others are flaky. One already named was confirmed by the run that did.
            before = commits[commits.index(culprit) - 1] if commits.index(culprit) else since
            unconfirmed = [name for name in hit if state["named"].get(name) != culprit]
            # Each rerun's flaky harnesses are settled before the next rerun, which the budget may end.
            flaky = []
            for commit, fail_means_flaky in ((culprit, False), (before, True)):
                failed_again = fails(commit, [name for name in unconfirmed if name not in flaky], again=True)
                found = [name for name in unconfirmed if name not in flaky and (name in failed_again) == fail_means_flaky]
                if found:
                    report(f"parity-main: {', '.join(found)} {'fail' if fail_means_flaky else 'pass'} when run again "
                           f"at {commit[:9]}, {'before' if fail_means_flaky else 'the merge the search ended on'}"
                           f"{f' {culprit[:9]}' if fail_means_flaky else ''}: flaky: no merge is named.")
                    # Their single runs are no ground for the next run's search either.
                    for ran in results.values():
                        for name in found:
                            ran.pop(name, None)
                    BISECT.write_text(json.dumps(state), encoding="utf-8")
                    remaining = [name for name in remaining if name not in found]
                    flaky += found
            if len(flaky) < len(hit):
                blames.append((culprit, [name for name in hit if name not in flaky]))
            remaining = [name for name in remaining if name not in hit]
            low = commits.index(culprit) + 1
    except OutOfTime:
        for name in remaining:
            ran = [(index, results[commit][name]) for index, commit in enumerate(commits)
                   if name in results.get(commit, {})]
            bad = next(index for index, failed_there in ran if failed_there and index >= low)
            good = max([index for index, failed_there in ran if not failed_there and low <= index < bad], default=low - 1)
            report(f"parity-main: {name} passes at {(commits[good] if good >= 0 else since)[:9]} and fails at "
                   f"{commits[bad][:9]}: the search's {budget} minutes ended with {bad - good} merges left. "
                   "The next run goes on from there.")
    finally:
        git("checkout", "-q", "--detach", head)
    for culprit, hit in blames:
        subject = git("log", "-1", "--format=%s", culprit).stdout.strip()
        number = re.search(r"Merge pull request #(\d+)", subject) or re.search(r"\(#(\d+)\)$", subject)
        report(f"parity-main: {', '.join(hit)} first fail at {culprit[:9]} ({subject}), "
               f"of {len(commits)} merges since {since[:9]}.")
        hit = [name for name in hit if state["named"].get(name) != culprit]
        if comment and number and hit:
            url = f"{os.environ.get('GITHUB_SERVER_URL', 'https://github.com')}/{REPO}/actions/runs/{os.environ.get('GITHUB_RUN_ID', '')}"
            body = (f"🤖 Agent: parity-main found that this merge breaks {', '.join(f'`{name}`' for name in hit)} on main "
                    f"(bisected over {len(commits)} merges: {url}). Releases wait until parity-main passes again.")
            if subprocess.run(["gh", "pr", "comment", number.group(1), "--repo", REPO, "--body", body]).returncode == 0:
                state["named"].update({name: culprit for name in hit})
                BISECT.write_text(json.dumps(state), encoding="utf-8")
    return 1


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("base")
    commands.add_parser("history")
    judging = commands.add_parser("verdict")
    judging.add_argument("since", help="the last commit known to pass (the run's base)")
    judging.add_argument("--timeout", type=int, default=2400, help="seconds per harness")
    bisecting = commands.add_parser("bisect")
    bisecting.add_argument("since", help="the last commit known to pass (the run's base)")
    bisecting.add_argument("--timeout", type=int, default=2400, help="seconds per harness")
    bisecting.add_argument("--comment", action="store_true", help="comment on the breaking merge's pull request")
    bisecting.add_argument("--budget", type=int, default=90, help="minutes for the search; then it says how far it got")
    args = parser.parse_args()
    if args.command == "base":
        print(base())
        return 0
    if args.command == "history":
        return history()
    if args.command == "verdict":
        return verdict(args.since, args.timeout)
    return bisect(args.since, args.timeout, args.comment, args.budget)


if __name__ == "__main__":
    sys.exit(main())
