import contextlib
import copy
import datetime as dt
import json
import os
import subprocess
import tempfile
import unittest
from unittest import mock
from pathlib import Path

from tools.ci import nightly
from tools.ci import nightly_set as ns
from tests.test_ci_concurrency import yaml

NOW = "2026-10-28T12:00:00+00:00"
PATTERNS = ns.guarded_patterns((Path(__file__).resolve().parents[1] / ".github/CODEOWNERS").read_text())


def pr(n, head="h", state="open", files=("crates/horizon-world/src/lib.rs",), base="main"):
    return ns.PullRequest(n, state, f"{head}{n}", f"Fix {n}", f"dev{n}", base, list(files))


def lookup(*prs):
    known = {p.number: p for p in prs}
    return lambda n: known.get(n)


def run(state, text, login="kacpherek", *prs):
    return ns.apply(state, ns.parse(text), ns.Actor(login, 101 if login == "kacpherek" else 9999), NOW,
                    lookup(*prs), PATTERNS)

T = dt.datetime(2026, 10, 28, 12, 0, tzinfo=dt.timezone.utc)


def merged_pr(n, title, labels, issues=(), paths=("crates/horizon-world/src/lib.rs",)):
    return {"number": n, "title": title, "labels": labels, "issues": list(issues), "author": f"dev{n}",
            "paths": list(paths)}


def notes(chosen=None, picked=(), left_out=(), merged=(), issues=()):
    chosen = chosen or nightly.Ref("main", "b" * 40, T)
    return nightly.notes("2026-10-28", chosen, nightly.Ref("main", "m" * 40, T), list(picked), list(left_out),
                         list(merged), list(issues))


class NightlyTests(unittest.TestCase):
    def test_only_old_dated_nightlies_are_pruned(self):
        tags = [{"tagName": "nightly", "isPrerelease": True},
                {"tagName": "nightly-2026-10-20", "isPrerelease": True},
                {"tagName": "nightly-2026-10-21", "isPrerelease": True},
                {"tagName": "nightly-2026-10-19", "isPrerelease": False},
                {"tagName": "v0.2.80", "isPrerelease": False},
                {"tagName": "nightly-2026-10-15-extra", "isPrerelease": True}]
        self.assertEqual(nightly.stale(tags, dt.date(2026, 10, 28), 7), ["nightly-2026-10-20"])

    def test_a_negative_retention_window_is_refused_and_today_is_never_pruned(self):
        tags = [{"tagName": f"nightly-2026-11-{day:02d}", "isPrerelease": True} for day in range(10, 18)]
        today = dt.date(2026, 11, 17)
        self.assertEqual(nightly.stale(tags, today, 7), [])
        # Even at a zero-day window the cutoff is today, so today's nightly is never older than it.
        self.assertNotIn("nightly-2026-11-17", nightly.stale(tags, today, 0))
        # A negative window used to move the cutoff into the future and doom every tag, today's included.
        with self.assertRaisesRegex(ValueError, "non-negative"):
            nightly.stale(tags, today, -1)

    def test_notes_list_what_reached_main_as_a_release_did(self):
        merged = [merged_pr(1, "Fix pistons", ["feature"], issues=[7]), merged_pr(2, "Water flows", []),
                  merged_pr(3, "CI only", ["skip-release"]), merged_pr(4, "Docs", [], paths=[])]
        text = notes(merged=merged, issues=["- Known thing"])
        self.assertIn("Nightly 2026-10-28 · bbbbbbb: main (mmmmmmm), built from `main`", text)
        self.assertIn("### New\n- Fix pistons (#1, fixes #7)", text)
        self.assertIn("### Fixed\n- Water flows (#2)", text)
        self.assertNotIn("CI only", text)
        self.assertNotIn("Docs", text)
        self.assertIn("### Known issues\n- Known thing", text)
        self.assertIn("Thanks to @dev1, @dev2.", text)
        self.assertNotIn("### Not merged yet", text)
        self.assertIn("Nothing new for players on main", notes())

    def test_notes_open_with_the_way_to_the_launcher(self):
        # A raw HTML line that launchers skip, as on a game release.
        with mock.patch.object(nightly.train, "REPO", "horizon-rust/horizon"):
            self.assertTrue(notes().startswith(nightly.train.PLAYERS + "\n\nNightly 2026-10-28 · bbbbbbb: "))
            self.assertIn(nightly.PLAYERS, notes())

    def test_other_repositories_keep_the_upstream_intro(self):
        with mock.patch.object(nightly.train, "REPO", "another/horizon"):
            self.assertTrue(notes().startswith(nightly.train.PLAYERS + "\n\n"))
            self.assertIn(nightly.PLAYERS, notes())

    def test_notes_name_the_commit_and_its_permanent_source_tag(self):
        sha = "0c0206714abc" + "0" * 28
        text = notes(chosen=nightly.Ref("main", sha, None))
        self.assertIn(f"Source: commit {sha}, kept for good as the tag `nightly-source/2026-10-28-0c02067`", text)

    def test_the_source_tag_outlives_every_prune(self):
        tag = nightly.source_tag("2026-10-20", "a" * 40)
        self.assertEqual(nightly.stale([{"tagName": tag, "isPrerelease": True}], dt.date(2026, 11, 28), 7), [])
        self.assertIsNone(nightly.DATED.match(tag))


class RotateTests(unittest.TestCase):
    def test_fallback_tag_uses_utc_not_the_local_date(self):
        # A runner west of UTC, after 17:00 UTC: the two calendars disagree by a day.
        utc_today, local_today = dt.date(2026, 11, 17), dt.date(2026, 11, 16)
        now_timezones = []

        class FakeDateTime(dt.datetime):
            @classmethod
            def now(cls, tz=None):
                now_timezones.append(tz)
                return dt.datetime(utc_today.year, utc_today.month, utc_today.day, 12, 0, 0,
                                   tzinfo=(tz or dt.timezone.utc))

        class FakeDate(dt.date):
            @classmethod
            def today(cls):
                return local_today

        fake_dt = type("FakeDateTimeModule", (), {
            "datetime": FakeDateTime, "date": FakeDate,
            "timedelta": dt.timedelta, "timezone": dt.timezone,
        })

        calls = []
        def fake_gh(*args, **kwargs):
            calls.append(args)
            if args[:3] == ("release", "view", "nightly"):
                return json.dumps({"name": "Nightly - no date in this title"})
            return ""

        with mock.patch.object(nightly, "gh", fake_gh), \
                mock.patch.object(nightly, "dt", fake_dt), \
                mock.patch.dict(nightly.os.environ, {"GITHUB_REPOSITORY": "horizon-rust/horizon"}):
            nightly.rotate()

        rename = next(a for a in calls if a[:3] == ("release", "edit", "nightly"))
        self.assertEqual(rename[-1], "nightly-" + str(utc_today - dt.timedelta(days=1)))
        self.assertNotEqual(rename[-1], "nightly-" + str(local_today - dt.timedelta(days=1)))
        self.assertEqual(now_timezones, [dt.timezone.utc])


class ScheduledTests(unittest.TestCase):
    def test_the_scheduled_nightly_drops_merged_closed_and_refused_ones(self):
        state = run({}, "/nightly 1 2 3 4", "kacpherek", pr(1), pr(2), pr(3), pr(4)).state
        get = lookup(pr(1, head="new"), pr(2, state="merged"), pr(3, state="closed"),
                     pr(4, files=[".github/workflows/x.yml"]))
        kept, left_out = nightly.refresh(state["prs"], get, PATTERNS)
        self.assertEqual([(e["number"], e["head"]) for e in kept], [(1, "h1")])
        self.assertEqual(len(left_out), 3)
        self.assertIn("merged", left_out[0])

    def test_the_scheduled_nightly_builds_the_head_that_was_picked(self):
        state = run({}, "/nightly 1", "kacpherek", pr(1)).state
        kept, left_out = nightly.refresh(state["prs"], lookup(pr(1, head="new")), PATTERNS)
        self.assertEqual([(e["number"], e["head"]) for e in kept], [(1, "h1")])
        self.assertEqual(left_out, [])

    def test_an_unreviewed_pull_request_stays_in_the_scheduled_set(self):
        # No review is looked at: a pull request nobody reviewed is kept as long as nothing else refuses it.
        state = run({}, "/nightly 1", "kacpherek", pr(1)).state
        kept, left_out = nightly.refresh(state["prs"], lookup(pr(1)), PATTERNS)
        self.assertEqual([e["number"] for e in kept], [1])
        self.assertEqual(left_out, [])

    def test_notes_list_each_pull_request_and_who_picked_it(self):
        prs = run({}, "/nightly 1 2", "kacpherek", pr(1), pr(2)).state["prs"]
        text = notes(chosen=nightly.Ref("nightly/build-2026-10-28-1", "b" * 40, None), picked=prs,
                     left_out=["#3 Fix 3: closed, left out"])
        self.assertIn("main (mmmmmmm) with 2 pull requests merged in", text)
        self.assertIn("### Not merged yet (2)\n- Fix 1 (#1) by dev1, at h1, picked by kacpherek", text)
        self.assertIn("### Left out (1)", text)

if __name__ == "__main__":
    unittest.main()


WORKFLOWS = Path(__file__).resolve().parents[1] / ".github" / "workflows"


class FirstNightlyTests(unittest.TestCase):
    def test_fetch_succeeds_with_or_without_an_existing_nightly_tag(self):
        class ReachedGreenBase(Exception):
            pass

        for existing in (False, True):
            with self.subTest(existing=existing):
                calls = []
                def git(*args):
                    calls.append(args)
                    if args[0] == "ls-remote":
                        return "abc\\trefs/tags/nightly" if existing else ""
                    return ""
                with mock.patch.object(nightly, "git", side_effect=git), mock.patch.object(nightly, "green_base", side_effect=ReachedGreenBase):
                    with self.assertRaises(ReachedGreenBase):
                        nightly.pick(False)
                fetched = next(args for args in calls if args[0] == "fetch")
                self.assertIn("+refs/heads/main:refs/remotes/origin/main", fetched)
                self.assertEqual("+refs/tags/nightly:refs/tags/nightly" in fetched, existing)


def polytone_gate_bypasses(game, nightly_build):
    """Where the Polytone tests could fail without holding the Nightly (game-build.yml's `polytone-windows`, called
    through nightly-build.yml's `build`)."""
    problems = []
    job = game["jobs"]["polytone-windows"]
    for where, part in [("job", job), *((f"step {i}", step) for i, step in enumerate(job["steps"]))]:
        problems += [f"polytone-windows {where}: {key}" for key in ("if", "continue-on-error") if key in part]
    for name in ("sign", "publish", "sdk-publish"):
        gate = nightly_build["jobs"][name]
        if "build" not in gate["needs"]:
            problems.append(f"{name} doesn't need build")
        condition = "".join(str(gate.get("if", "")).split())
        problems += [f"{name} runs on {status}" for status in ("always()", "cancelled()", "failure()") if status in condition]
    return problems


class UntrustedBuildTests(unittest.TestCase):
    """A nightly can hold pull requests nobody reviewed: nothing its build leaves may reach a release (game-build.yml)."""

    def setUp(self):
        self.game = (WORKFLOWS / "game-build.yml").read_text()

    def test_no_nightly_build_is_trusted(self):
        # The train only starts `nightly build` (Nightly only since 2026-10-28); it builds nothing itself.
        release = (WORKFLOWS / "release.yml").read_text()
        self.assertNotIn("/game-build.yml", release)
        self.assertIn("gh workflow run nightly-build.yml", release)
        nightly_build = (WORKFLOWS / "nightly-build.yml").read_text()
        self.assertIn("trusted: false", nightly_build)
        self.assertNotIn("trusted: true", nightly_build)
        self.assertIn("default: false", self.game.split("trusted:", 1)[1].split("developer_id:", 1)[0])

    def test_an_untrusted_build_neither_shares_nor_saves_the_release_cache(self):
        self.assertIn("shared-key: ${{ inputs.trusted && 'release' || 'nightly' }}-${{ matrix.name }}", self.game)
        self.assertIn("save-if: ${{ inputs.trusted }}", self.game)
        self.assertEqual(self.game.count("Swatinem/rust-cache"), 1)

    def test_an_untrusted_build_never_runs_on_the_release_runner(self):
        runs_on = [line for line in self.game.splitlines() if "runs-on:" in line]
        self.assertEqual(len(runs_on), 4)
        # The host-independent wasm member and the Polytone tests run on credential-free hosted runners.
        hosted = [line for line in runs_on if line.strip() in ("runs-on: ubuntu-22.04", "runs-on: windows-latest")]
        self.assertEqual(len(hosted), 2)
        # The nightly's own runner: never the release runner, and never behind pull requests' parity queue.
        nightly_runner = "fromJSON('[\"self-hosted\", \"macOS\", \"horizon-nightly\"]')"
        for line in runs_on:
            if line in hosted:
                continue
            self.assertIn(f"inputs.trusted && vars.MAC_RUNNER || {nightly_runner}", line)

    def test_the_polytone_tests_hold_the_nightly_as_the_windows_build_does(self):
        # Their own job beside the Windows build: whatever it was given, a failure must still fail the called
        # workflow, which sign, publish and sdk-publish need.
        game = yaml.safe_load(self.game)
        nightly_build = yaml.safe_load((WORKFLOWS / "nightly-build.yml").read_text())
        job = game["jobs"]["polytone-windows"]
        self.assertEqual(job["runs-on"], "windows-latest")
        self.assertEqual(job["env"]["RUSTFLAGS"], "-C target-feature=+crt-static")
        self.assertEqual(job["steps"][-1]["run"].strip(),
                         "cargo test --release --locked -p horizon-client-render --lib particles::polytone::tests::")
        self.assertEqual(polytone_gate_bypasses(game, nightly_build), [])
        # Each way around the gate is caught: a step or job allowed to fail, a skipped test, a publishing job that
        # runs after a failed build.
        for bypass in ({"step": {"continue-on-error": True}}, {"step": {"if": "github.event_name == 'schedule'"}},
                       {"job": {"continue-on-error": True}}, {"job": {"if": "${{ inputs.trusted }}"}},
                       {"sign": {"if": "${{ !cancelled() }}"}}, {"publish": {"if": "always() && true"}},
                       {"sdk-publish": {"if": "failure() || needs.sdk.outputs.needed == 'true'"}},
                       {"sign": {"needs": ["pick", "sdk-index"]}}):
            with self.subTest(bypass=bypass):
                mutated_game, mutated_build = copy.deepcopy(game), copy.deepcopy(nightly_build)
                [(where, change)] = bypass.items()
                polytone = mutated_game["jobs"]["polytone-windows"]
                {"step": polytone["steps"][-1], "job": polytone}.get(where, mutated_build["jobs"].get(where)).update(change)
                self.assertNotEqual(polytone_gate_bypasses(mutated_game, mutated_build), [])

    def test_a_trusted_build_never_reads_the_shared_sccache(self):
        calls = self.game.count("bash tools/ci/runner-env.sh")
        self.assertEqual(self.game.count("HORIZON_NO_SCCACHE: ${{ inputs.trusted && '1' || '' }}"), calls)
        env = (Path(__file__).resolve().parents[1] / "tools/ci/runner-env.sh").read_text()
        self.assertEqual(env.count('-z "${HORIZON_NO_SCCACHE:-}"'), 2)


class PruneBuildsTests(unittest.TestCase):
    TODAY = dt.date(2026, 11, 10)

    def test_only_old_unpublished_builds_go(self):
        branches = {
            "nightly/build-2026-10-27-1": "a",   # old: goes
            "nightly/build-2026-10-27-2": "pub",  # old, but a published nightly-<date> points to it: stays
            "nightly/build-2026-11-02-1": "c",   # old enough by one day: goes
            "nightly/build-2026-11-03-1": "d",   # exactly 7 days: stays
            "nightly/build-2026-11-09-3": "e",   # recent: stays
        }
        self.assertEqual(nightly.stale_builds(branches, {"pub"}, self.TODAY, 7),
                         ["nightly/build-2026-10-27-1", "nightly/build-2026-11-02-1"])

    def test_the_newest_build_always_stays(self):
        branches = {"nightly/build-2026-10-01-1": "a", "nightly/build-2026-10-01-2": "b"}
        self.assertEqual(nightly.stale_builds(branches, set(), self.TODAY, 7), ["nightly/build-2026-10-01-1"])
        self.assertEqual(nightly.stale_builds({"nightly/build-2026-10-01-1": "a"}, set(), self.TODAY, 7), [])
        # the newest by number, not by text: -10 is newer than -9
        branches = {"nightly/build-2026-10-01-9": "a", "nightly/build-2026-10-01-10": "b"}
        self.assertEqual(nightly.stale_builds(branches, set(), self.TODAY, 7), ["nightly/build-2026-10-01-9"])

    def test_nothing_else_is_touched(self):
        branches = {"nightly-state": "s", "nightly/build-x": "x", "main": "m", "nightly/build-2026-10-01-1-fix": "f",
                    "feature/nightly/build-2026-10-01-1": "g", "nightly/build-2026-10-02-1": "n"}
        self.assertEqual(nightly.stale_builds(branches, set(), self.TODAY, 7), [])

    def test_the_source_is_archived_from_git_signed_attached_and_tagged(self):
        build = (WORKFLOWS / "nightly-build.yml").read_text()
        pick = build.split("\n  build:", 1)[0]
        self.assertIn('git archive --format=tar.gz --prefix="horizon-${SHA:0:7}/" -o horizon-source.tar.gz "$SHA"',
                      pick)
        sign = build.split("\n  sign:", 1)[1].split("\n  publish:", 1)[0]
        self.assertIn("dist/horizon-source.tar.gz", sign.split("sign_release.sh nightly", 1)[1])
        publish = build.split("\n  publish:", 1)[1].split("\n  answer:", 1)[0]
        self.assertIn('"dist/horizon-source.tar.gz#Source of this Nightly', publish)
        self.assertIn('nightly.py keep-source "$SHA" "$DATE"', publish)

    def test_the_workflow_prunes_on_schedule_only(self):
        build = (WORKFLOWS / "nightly-build.yml").read_text()
        step = build.split("Old nightly/build-* branches deleted", 1)[1].split("- uses:", 1)[0]
        self.assertIn("if: github.event_name == 'schedule'", step)
        self.assertIn("prune-builds --days 7", step)
        self.assertIn("GITHUB_STEP_SUMMARY", step)


class ConcurrencyTests(unittest.TestCase):
    """A dry run never waits for, holds up or replaces a real cut or another dry run (#18779)."""

    def setUp(self):
        self.build = (WORKFLOWS / "nightly-build.yml").read_text()
        # The rule `pick` publishes by: a dry run is the `dry_run` input or any dispatch off main.
        mode = self.build.split("\n  pick:", 1)[1].split("- id: mode", 1)[1].split("- id: pick", 1)[0]
        self.dry = mode.split('run: echo "dry_run=${{ ', 1)[1].split(" }}", 1)[0]

    def test_real_runs_keep_the_shared_group_and_dry_runs_get_one_each(self):
        concurrency = self.build.split("\nconcurrency:\n", 1)[1].split("\njobs:", 1)[0]
        settings = [line.strip() for line in concurrency.splitlines() if not line.strip().startswith("#")]
        # Not a dry run: the suffix is '', so the group is `nightly-build` as before, never cancelling.
        self.assertEqual(settings, [
            f"group: nightly-build${{{{ ({self.dry}) && format('-dry-{{0}}', github.run_id) || '' }}}}",
            "cancel-in-progress: false",
        ])
        # Written out, so a rule changed in all three places at once still fails here.
        self.assertEqual(self.dry, "github.event_name == 'workflow_dispatch' && "
                                   "(inputs.dry_run || github.ref != 'refs/heads/main')")

    def test_the_run_name_says_dry_run_or_real_and_keeps_the_request(self):
        run_name = self.build.split("\nrun-name: ", 1)[1].split("\n", 1)[0]
        # nightly_set.dispatch finds the run it started by the request id right after the mode, before the free-form
        # base (nightly_set.title_request).
        self.assertEqual(run_name, "nightly build"
                         f"${{{{ ({self.dry}) && ' (dry run)' || ' (real)' }}}}"
                         "${{ inputs.request && format(' (request {0})', inputs.request) || '' }}"
                         "${{ inputs.base && format(' on {0}', inputs.base) || '' }}"
                         "${{ inputs.off_main && ' (off main)' || '' }}")

    def test_dispatch_finds_its_own_run_not_one_whose_base_names_its_request(self):
        runs = [
            {"displayTitle": "nightly build (real) on 2640-1795140000 (request 2640-1795140000)", "url": "u/base"},
            {"displayTitle": "nightly build (dry run) on 2640-1795140000", "url": "u/no-request"},
            {"displayTitle": "nightly build (real) (request 2640-17951400001)", "url": "u/longer-id"},
            {"displayTitle": "nightly build (dry run) (request 2640-1795140000) on abc", "url": "u/ours"},
        ]

        def gh(*args, check=True):
            return json.dumps(runs) if args[:2] == ("run", "list") else ""

        with mock.patch.object(ns, "gh", side_effect=gh), mock.patch.object(ns.time, "sleep"):
            self.assertEqual(ns.dispatch("o/r", "2640", "2640-1795140000", []), "u/ours")
        self.assertEqual(ns.title_request("nightly build (real) (request 7-1) on 2640-1795140000"), "7-1")
        self.assertEqual(ns.title_request("nightly build (real) on (request 7-1)"), "")



class SdkPairTests(unittest.TestCase):
    def test_only_complete_published_exact_pair_is_eligible(self):
        # Written out, not taken from the module: the release a Nightly needs, signed index and sums included.
        assets = [{"name": name} for name in (
            "sdk.json", "SHA256SUMS", "SHA256SUMS.sig", "api-source.tar.xz", "build-data.tar.xz", "surface.json",
            *(f"{kind}-{leg}.tar.xz" for kind in ("runtime", "api", "engine") for leg in (
                "x86_64-unknown-linux-gnu", "x86_64-unknown-linux-gnu-server", "x86_64-pc-windows-gnu",
                "aarch64-apple-darwin")))]
        release = {"draft": False, "tag_name": "sdk-" + "a" * 40, "assets": assets}
        for changes, expected in [({}, True), ({"draft": True}, False),
                                  ({"tag_name": "sdk-" + "b" * 40}, False),
                                  # The old prebuilt-<sha> release is not an SDK.
                                  ({"tag_name": "prebuilt-" + "a" * 40}, False)]:
            with mock.patch.object(nightly.train, "gh_json", return_value=[[], [{**release, **changes}]]):
                self.assertEqual(nightly.sdk_ready("a" * 40), expected)
        for index in range(len(assets)):
            with mock.patch.object(nightly.train, "gh_json", return_value=[[{
                    **release, "assets": assets[:index] + assets[index + 1:]}]]):
                self.assertFalse(nightly.sdk_ready("a" * 40))
        with mock.patch.object(nightly.train, "gh_json", return_value=[[]]):
            self.assertFalse(nightly.sdk_ready("a" * 40))
        with mock.patch.object(nightly.train, "gh_json", side_effect=SystemExit("API failed")):
            with self.assertRaises(SystemExit):
                nightly.sdk_ready("a" * 40)

    def test_the_base_is_mains_green_commit_and_needs_no_sdk_yet(self):
        # The SDK of the Nightly's commit is built after this pick (nightly-build.yml), so none is looked for here.
        with mock.patch.dict(nightly.os.environ, {"NIGHTLY_BASE": ""}), \
                mock.patch.object(nightly.train, "green_main_head", return_value="a" * 40) as automatic, \
                mock.patch.object(nightly, "sdk_ready", side_effect=AssertionError("asked")), \
                mock.patch.object(nightly, "is_ancestor", return_value=True), \
                mock.patch.object(nightly, "git", side_effect=["a" * 40, NOW]):
            self.assertEqual(nightly.green_base("horizon-rust/horizon", "").sha, "a" * 40)
            automatic.assert_called_once_with(parity=True)
        with mock.patch.dict(nightly.os.environ, {"NIGHTLY_BASE": "b" * 40}), \
                mock.patch.object(nightly.train, "green_main_head") as automatic, \
                mock.patch.object(nightly, "is_ancestor", return_value=False), \
                mock.patch.object(nightly, "stop", side_effect=SystemExit):
            with self.assertRaises(SystemExit):
                nightly.green_base("horizon-rust/horizon", "")
            automatic.assert_not_called()

    def test_a_final_source_that_is_not_mains_commit_stops_before_pushing(self):
        with mock.patch.dict(nightly.os.environ, {"GITHUB_REPOSITORY": "horizon-rust/horizon"}), \
                mock.patch.object(nightly.ns, "fetch_heads", return_value=[]), \
                mock.patch.object(nightly.ns, "assemble", return_value="b" * 40), \
                mock.patch.object(nightly.ns, "push") as push:
            with self.assertRaises(SystemExit):
                nightly.build_set(nightly.Ref("main", "a" * 40, T), [], [], "2026-11-11", True, False)
            push.assert_not_called()

    def test_publication_checks_fresh_inventory_for_the_current_repository(self):
        with mock.patch.dict(nightly.os.environ, {"GITHUB_REPOSITORY": "horizon-rust/horizon"}), \
                mock.patch.object(nightly, "ready_sdk_shas", side_effect=[{"a" * 40}, set()]) as inventory, \
                mock.patch.object(nightly, "published_index") as index:
            nightly.require_sdk("a" * 40)
            index.assert_called_once_with("a" * 40)
            with self.assertRaisesRegex(SystemExit, "publication refused"):
                nightly.require_sdk("a" * 40)
            self.assertEqual(inventory.call_count, 2)
        with mock.patch.object(nightly.train, "REPO", "someone/horizon"), \
                mock.patch.object(nightly, "sdk_ready", return_value=True) as sdk, \
                mock.patch.object(nightly, "published_index") as index:
            nightly.require_sdk("a" * 40)
            sdk.assert_called_once_with("a" * 40)
            index.assert_called_once_with("a" * 40)

    def test_the_sdk_index_must_name_this_build_format_and_assets_and_match_its_sums(self):
        import hashlib
        sha = "a" * 40
        good = {"build": {"commit": sha}, "package_format": nightly.package_format(),
                "assets": {name: {} for name in nightly.SDK_ARCHIVES}}
        self.assertEqual(nightly.checked_index(sha, json.dumps(good).encode()), good)
        for why, document in (
                ("for build", {**good, "build": {"commit": "b" * 40}}),
                ("package format", {**good, "package_format": 0}),
                ("does not list surface.json", {**good, "assets": {name: {} for name in nightly.SDK_ARCHIVES[:-1]}}),
                ("not an SDK index", {"build": {}})):
            with self.subTest(why), self.assertRaisesRegex(SystemExit, why):
                nightly.checked_index(sha, json.dumps(document).encode())
        sums = f"{hashlib.sha256(b'index').hexdigest()}  sdk.json\n{'0' * 64} *surface.json\n"
        self.assertTrue(nightly.listed("sdk.json", b"index", sums))
        self.assertFalse(nightly.listed("sdk.json", b"other", sums))
        self.assertFalse(nightly.listed("surface.json", b"index", sums))
        self.assertFalse(nightly.listed("SHA256SUMS", b"index", sums))

    def test_an_sdk_asset_is_trusted_only_under_the_forks_signature_for_its_own_tag(self):
        command = nightly.signature_check("a" * 40, Path("/tmp/x"), ["sdk.json"])
        self.assertEqual(command[:6], ["cargo", "run", "--locked", "--quiet", "-p", "horizon-release-sign"])
        arguments = command[command.index("--") + 1:]
        self.assertEqual(arguments, ["verify", "--repository", "horizon-rust/horizon", "--tag", "sdk-" + "a" * 40,
                                     "--only", "sdk.json", "/tmp/x"])
        self.assertNotIn("--publishing", command, "an unsigned SDK is refused, as the launcher refuses it")

    def test_the_previous_sdk_is_the_nearest_published_one_behind_the_build(self):
        a, b, c, d = ("a" * 40, "b" * 40, "c" * 40, "d" * 40)
        # c is not on d's history; b is nearer than a.
        with mock.patch.object(nightly, "ready_sdk_shas", return_value={a, b, c, d}), \
                mock.patch.object(nightly, "is_ancestor", side_effect=lambda x, y: x in (a, b)), \
                mock.patch.object(nightly, "git", side_effect=lambda *args: {f"{a}..{d}": "9", f"{b}..{d}": "2"}[args[-1]]):
            self.assertEqual(nightly.previous_sdk(d), b)
        with mock.patch.object(nightly, "ready_sdk_shas", return_value={d}):
            self.assertEqual(nightly.previous_sdk(d), "")

    def test_fork_pick_refuses_explicit_prs_ignores_saved_prs_and_upstream_keeps_both(self):
        main = nightly.Ref("main", "a" * 40, T)
        saved = [{"number": 17, "head": "b" * 40}]
        explicit = [{"number": 19, "head": "c" * 40}]
        for repo in ("horizon-rust/horizon", "another/fork"):
            for spec in ("", "19"):
                with self.subTest(repo=repo, spec=spec), \
                        mock.patch.dict(nightly.os.environ, {"GITHUB_REPOSITORY": repo, "NIGHTLY_PRS": spec,
                                                            "GITHUB_EVENT_NAME": "workflow_dispatch", "DRY_RUN": "true"}), \
                        mock.patch.object(nightly, "git", return_value=""), \
                        mock.patch.object(nightly, "green_base", return_value=main), \
                        mock.patch.object(nightly.ns, "load_state", return_value=({"prs": saved}, "", "")) as state, \
                        mock.patch.object(nightly.ns, "lookup"), \
                        mock.patch.object(nightly, "requested", return_value=explicit) as requested, \
                        mock.patch.object(nightly, "build_set") as build:
                    if repo == "horizon-rust/horizon" and spec:
                        with self.assertRaises(SystemExit):
                            nightly.pick(False)
                        build.assert_not_called()
                        requested.assert_not_called()
                        state.assert_not_called()
                    else:
                        nightly.pick(False)
                        expected = [] if repo == "horizon-rust/horizon" else explicit if spec else saved
                        self.assertEqual(build.call_args.args[:3], (main, expected, []))
                        if repo == "horizon-rust/horizon":
                            state.assert_not_called()
                            requested.assert_not_called()


def repository(folder: Path) -> tuple[Path, dict[str, str]]:
    """An origin with main, a dry-run/* branch, ordinary branches (a person's fix, a fork snapshot under union/, a
    Dependabot bump), refs/pull/1/head (a fork's pull request), a Nightly build branch and a merge queue branch,
    each with a commit of its own; and a clone that has every one of those commits locally, as a fork's commit is
    there for anyone who asks GitHub for its SHA."""
    env = {**os.environ, "GIT_AUTHOR_NAME": "t", "GIT_AUTHOR_EMAIL": "t@t", "GIT_COMMITTER_NAME": "t",
           "GIT_COMMITTER_EMAIL": "t@t", "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": os.devnull}

    def run(*args, cwd=folder):
        return subprocess.run(["git", *args], cwd=cwd, env=env, check=True, capture_output=True, text=True,
                              input="").stdout.strip()

    origin = str(folder / "origin")
    run("init", "-q", "--bare", origin)
    tree = run("--git-dir", origin, "mktree")
    main = run("--git-dir", origin, "commit-tree", tree, "-m", "main")
    run("--git-dir", origin, "update-ref", "refs/heads/main", main)
    run("--git-dir", origin, "symbolic-ref", "HEAD", "refs/heads/main")
    shas = {"main": main}
    for name, ref in [("branch", "refs/heads/dry-run/release-path"), ("fix", "refs/heads/fix/release-path"),
                      ("union", "refs/heads/union/x"), ("dependabot", "refs/heads/dependabot/cargo/x"),
                      ("pull", "refs/pull/1/head"),
                      ("nightly", "refs/heads/nightly/build-2026-11-20-1"),
                      ("queue", "refs/heads/gh-readonly-queue/main/pr-1-abc")]:
        shas[name] = run("--git-dir", origin, "commit-tree", tree, "-p", main, "-m", name)
        run("--git-dir", origin, "update-ref", ref, shas[name])
    clone = folder / "clone"
    run("clone", "-q", origin, str(clone))
    run("fetch", "-q", "origin", "+refs/pull/1/head:refs/pull-1", "+refs/heads/*:refs/remotes/origin/*", cwd=clone)
    return clone, shas


class OffMainDryRunTests(unittest.TestCase):
    """A dry run may build a commit off main (`off_main`), so a fix to the release path is tried before it merges;
    never a real cut, never a fork's commit, never with a signing key."""

    def base(self, clone: Path, sha: str, dry: bool, off_main: bool):
        def stop(repo, issue, text):
            raise SystemExit(text)

        environ = {"NIGHTLY_BASE": sha, "DRY_RUN": "true" if dry else "false",
                   "NIGHTLY_OFF_MAIN": "true" if off_main else "false",
                   "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": os.devnull}
        with contextlib.chdir(clone), mock.patch.dict(os.environ, environ), \
                mock.patch.object(nightly, "stop", side_effect=stop):
            return nightly.green_base("horizon-rust/horizon", "")

    def test_only_a_dry_run_marked_off_main_builds_a_dry_run_branch(self):
        with tempfile.TemporaryDirectory() as folder:
            clone, shas = repository(Path(folder))
            self.assertEqual(self.base(clone, shas["branch"], dry=True, off_main=True).sha, shas["branch"])
            self.assertEqual(self.base(clone, shas["branch"][:12], dry=True, off_main=True).sha, shas["branch"])
            # A real cut keeps the on-main check, `off_main` or not.
            for off_main in (False, True):
                with self.assertRaisesRegex(SystemExit, "is not on main"):
                    self.base(clone, shas["branch"], dry=False, off_main=off_main)
            # A dry run off main without the input is refused as before: its run's name would not say so.
            with self.assertRaisesRegex(SystemExit, "is not on main"):
                self.base(clone, shas["branch"], dry=True, off_main=False)
            # The input with a base on main is refused too, so the name never says "off main" wrongly.
            with self.assertRaisesRegex(SystemExit, "off_main is set"):
                self.base(clone, shas["main"], dry=True, off_main=True)
            self.assertEqual(self.base(clone, shas["main"], dry=True, off_main=False).sha, shas["main"])

    def test_a_commit_no_dry_run_branch_holds_is_refused_though_the_clone_has_it(self):
        with tempfile.TemporaryDirectory() as folder:
            clone, shas = repository(Path(folder))
            # Each commit is in the clone (every branch and refs/pull/1/head were fetched), yet no dry-run/* branch
            # holds it: ordinary branches, a fork snapshot, Dependabot's, a pull request's head, the Nightly's own
            # build branch and the merge queue's. The refusal says what to do instead.
            for name in ("fix", "union", "dependabot", "pull", "nightly", "queue"):
                with self.subTest(name=name), self.assertRaisesRegex(
                        SystemExit, r"no dry-run/\* branch .* Push the commit to dry-run/<name>, dispatch the workflow "
                                    r"from main with base=<full sha> and off_main=true, and delete the branch"):
                    self.base(clone, shas[name], dry=True, off_main=True)
            for name in ("refs/pull-1", "f" * 40, "nonsense"):
                with self.subTest(name=name), self.assertRaises(SystemExit):
                    self.base(clone, name, dry=True, off_main=True)

    def test_an_off_main_base_cannot_reach_a_publishing_step_or_a_key(self):
        build = (WORKFLOWS / "nightly-build.yml").read_text()
        jobs = yaml.safe_load(build)["jobs"]
        dry = "needs.pick.outputs.dry_run != 'true'"
        # One dry-run value: the one `pick` checks an off-main base against is the one every later job reads.
        pick = jobs["pick"]
        step = next(s for s in pick["steps"] if s.get("id") == "pick")
        self.assertEqual(step["env"]["DRY_RUN"], "${{ steps.mode.outputs.dry_run }}")
        self.assertEqual(step["env"]["NIGHTLY_OFF_MAIN"], "${{ inputs.off_main }}")
        self.assertEqual(pick["outputs"]["dry_run"], "${{ steps.mode.outputs.dry_run }}")
        # Every step that publishes, tags, rotates or prunes runs only when it isn't a dry run.
        publishing = ("gh release", "nightly.py rotate", "keep-source", "nightly.py prune ", "git/refs")
        steps = 0
        for name, job in jobs.items():
            if name == "dry-run-cleanup":
                continue
            for s in job.get("steps", []):
                if any(word in s.get("run", "") for word in publishing):
                    steps += 1
                    self.assertIn(dry, str(s.get("if", "")) + str(job.get("if", "")), (name, s.get("name")))
        self.assertEqual(steps, 1)  # the publish step; prune-builds runs on a schedule only (no schedule now)
        # Every secret and the `release` environment only off a dry run, and sdk-publish only publishes then.
        for line in build.splitlines():
            if "secrets." in line or "'release'" in line:
                self.assertIn(dry, line)
        self.assertEqual(set(jobs["sdk-publish"]["secrets"]), {"RELEASE_SIGNING_KEY", "RELEASE_SIGNING_KEY_NEXT"})
        self.assertIn(dry, jobs["sdk-publish"]["with"]["publish"])
        sdk = yaml.safe_load((WORKFLOWS / "sdk-publish.yml").read_text())["jobs"]
        self.assertIn("inputs.publish", sdk["publish"]["if"])
        self.assertIn("inputs.publish", sdk["sign"]["environment"])
        keys = next(s for s in sdk["sign"]["steps"] if "RELEASE_SIGNING_KEY" in s.get("env", {}))["env"]
        for key in ("RELEASE_SIGNING_KEY", "RELEASE_SIGNING_KEY_NEXT"):
            self.assertIn("inputs.publish &&", keys[key])
        # A dry run's own branch goes at the end, whatever happened before, by its exact name: never in a real run,
        # never a pattern, never another run's.
        cleanup = jobs["dry-run-cleanup"]
        self.assertEqual(cleanup["if"], "always() && needs.pick.outputs.dry_run == 'true' && "
                                        "needs.pick.outputs.pushed != ''")
        self.assertEqual(set(cleanup["needs"]), set(jobs) - {"dry-run-cleanup"})
        self.assertEqual(cleanup["permissions"], {"contents": "write"})
        self.assertEqual(pick["outputs"]["pushed"], "${{ steps.pick.outputs.pushed }}")
        (clean,) = cleanup["steps"]
        self.assertEqual(clean["env"]["NAME"], "${{ needs.pick.outputs.pushed }}")
        self.assertIn('[[ "$NAME" =~ ^nightly/dry-run-${GITHUB_RUN_ID}-[0-9]+$ ]] ||', clean["run"])
        self.assertIn('gh api -X DELETE "repos/$GITHUB_REPOSITORY/git/refs/heads/$NAME"', clean["run"])
        self.assertNotIn("*", clean["run"].replace("[0-9]+", ""))
        # The build legs, which run the commit's code, get no secrets at all.
        for name in ("build", "sdk"):
            self.assertNotIn("secrets", jobs[name])


class PrebuiltGameSecretsTests(unittest.TestCase):
    """prebuilt-game.yml calls the same sdk-publish.yml as the Nightly: it too passes only the signing keys, and none
    in a dry run, whose commit need not be on main and whose index and verify jobs run that commit's scripts."""

    def test_only_a_real_run_passes_the_signing_keys(self):
        text = (WORKFLOWS / "prebuilt-game.yml").read_text()
        jobs = yaml.safe_load(text)["jobs"]
        self.assertNotIn("inherit", text)
        real = "needs.source.outputs.publish == 'true'"
        self.assertEqual(jobs["publish"]["secrets"], {
            key: f"${{{{ {real} && secrets.{key} || '' }}}}"
            for key in ("RELEASE_SIGNING_KEY", "RELEASE_SIGNING_KEY_NEXT")})
        self.assertEqual(jobs["publish"]["with"]["publish"], f"${{{{ {real} }}}}")
        # `publish` is true only off a dry run dispatched from main, after `authorize` passed on that exact commit.
        source = next(s for s in jobs["source"]["steps"] if s.get("id") == "source")
        self.assertEqual(source["env"]["PUBLISH"], "${{ !inputs.dry_run && github.ref == 'refs/heads/main' }}")
        self.assertIn('python3 tools/prebuilt_game.py authorize "$COMMIT"', source["run"])
        # The keys name sdk-publish's own interface, and nothing else in this workflow reads a secret.
        sdk = yaml.safe_load((WORKFLOWS / "sdk-publish.yml").read_text())
        self.assertEqual(set(sdk[True]["workflow_call"]["secrets"]), set(jobs["publish"]["secrets"]))
        for line in text.splitlines():
            if "secrets." in line:
                self.assertIn(real, line)
        self.assertNotIn("secrets", jobs["build"])
