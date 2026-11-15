import datetime as dt
import os
import subprocess
import textwrap
from pathlib import Path
import tempfile
import unittest
from unittest import mock

from tools import release_train as train

NOW = dt.datetime(2026, 10, 15, 12, 0, tzinfo=dt.timezone.utc)


def pr(number, title="Fix a thing", labels=(), paths=("crates/horizon-client/src/game.rs",), issues=(), author="ludovic111",
       merged=dt.timedelta(hours=9)):
    return {"number": number, "title": title, "labels": list(labels), "paths": list(paths), "issues": list(issues),
            "author": author, "url": f"https://github.com/horizon-rust/horizon/pull/{number}", "merged_at": NOW - merged}


def merged_node(number, commit):
    return {"number": number, "mergeCommit": {"oid": commit}, "title": " Fix a thing ",
            "url": f"https://github.com/arismoko/horizon/pull/{number}",
            "mergedAt": (NOW - dt.timedelta(hours=9)).isoformat(), "author": {"login": "contributor"},
            "labels": {"nodes": []}, "closingIssuesReferences": {"nodes": []}}


def metadata_page(nodes):
    return {"data": {"repository": {"pullRequests": {"nodes": nodes}}}}


class ReleaseTrainTests(unittest.TestCase):
    def test_repository_merge_identity_before_metadata(self):
        for repository in ("arismoko/horizon", "horizon-rust/horizon"):
            with self.subTest(repository=repository), mock.patch.object(train, "REPO", repository), \
                    mock.patch.object(train, "run", return_value="root"), \
                    mock.patch.object(train, "merged_pull_requests", return_value=[
                        (27, "imported"), (28, "collision"), (28, "own")]), \
                    mock.patch.object(train, "gh_json", return_value=[metadata_page([]),
                        metadata_page([merged_node(28, "own")])]) as api, \
                    mock.patch.object(train, "changed_paths", return_value=["launcher/src/main.rs"]):
                last, prs = train.gather("launcher", "head", releases=[])
                self.assertIsNone(last)
                self.assertEqual([p["number"] for p in prs], [28])
                api.assert_called_once()
                args = api.call_args.args
                self.assertEqual(args[:4], ("api", "graphql", "--paginate", "--slurp"))
                self.assertIn(f"owner={repository.split('/')[0]}", args)
                self.assertIn("name=horizon", args)
                release, _ = train.decision("launcher", prs, NOW)
                self.assertTrue(release)
                _, body = train.notes("launcher", "launcher@0.2.0", prs, [])
                self.assertIn("#28", body)
                self.assertNotIn("#27", body)

    def test_first_fork_release_with_only_imported_history(self):
        with mock.patch.object(train, "run", return_value="root"), \
                mock.patch.object(train, "merged_pull_requests", return_value=[(27, "imported")]), \
                mock.patch.object(train, "gh_json", return_value=[metadata_page([])]):
            last, prs = train.gather("launcher", "head", releases=[])
            self.assertIsNone(last)
            self.assertEqual(prs, [])
            self.assertEqual(train.next_launcher_tag([], False, (0, 2)), "launcher@0.2.0")
            self.assertTrue(train.decision("launcher", prs, NOW, new_line="0.2")[0])
            self.assertFalse(train.decision("launcher", prs, NOW)[0])
            _, body = train.notes("launcher", "launcher@0.2.0", prs, [])
            self.assertNotIn("#27", body)

    def test_release_metadata_errors_are_not_imports(self):
        with mock.patch.object(train, "merged_pull_requests", return_value=[(28, "own")]), \
                mock.patch.object(train, "gh_json", side_effect=RuntimeError("API unavailable")):
            with self.assertRaisesRegex(RuntimeError, "API unavailable"):
                train.merged_between("launcher", "base", "head")
        broken = merged_node(28, "own")
        broken["mergedAt"] = "invalid date"
        with mock.patch.object(train, "merged_pull_requests", return_value=[(28, "own")]), \
                mock.patch.object(train, "gh_json", return_value=[metadata_page([broken])]):
            with self.assertRaises(ValueError):
                train.merged_between("launcher", "base", "head")

    def test_large_history_uses_paginated_metadata_without_per_pr_requests(self):
        nodes = [merged_node(n, f"sha-{n}") for n in range(1, 301)]
        nodes[0]["author"] = None
        nodes[0]["labels"]["nodes"] = [{"name": "bug"}]
        nodes[0]["closingIssuesReferences"]["nodes"] = [{"number": 9}, {"number": 3}]
        pages = [metadata_page(nodes[i:i + 100]) for i in range(0, 300, 100)]
        # Support the old N+1 implementation too: it returns the same records but must
        # fail the request-count assertion, rather than failing on a changed fixture shape.
        def response(*args):
            if args[1] == "graphql":
                return pages
            if args[0] == "api":
                return [[{"number": n["number"], "merge_commit_sha": n["mergeCommit"]["oid"],
                          "merged_at": n["mergedAt"]} for n in nodes]]
            node = nodes[int(args[2]) - 1].copy()
            node["labels"] = node["labels"]["nodes"]
            node["closingIssuesReferences"] = node["closingIssuesReferences"]["nodes"]
            return node
        with mock.patch.object(train, "merged_pull_requests", return_value=[
                (n["number"], n["mergeCommit"]["oid"]) for n in nodes]), \
                mock.patch.object(train, "gh_json", side_effect=response) as api, \
                mock.patch.object(train, "changed_paths", return_value=["launcher/src/main.rs"]):
            prs = train.merged_between("launcher", "base", "head")
        self.assertEqual(len(prs), 300)
        self.assertEqual(prs[0]["title"], "Fix a thing")
        self.assertEqual(prs[0]["issues"], [3, 9])
        self.assertEqual(prs[0]["labels"], ["bug"])
        self.assertEqual(prs[0]["author"], "")
        self.assertEqual(prs[-1]["number"], 300)
        self.assertEqual(prs[-1]["merged_at"], NOW - dt.timedelta(hours=9))
        api.assert_called_once()
        self.assertEqual(api.call_args.args[:4], ("api", "graphql", "--paginate", "--slurp"))

    def test_launcher_decision_shell_fails_closed_and_succeeds_normally(self):
        workflow = (train.ROOT / ".github/workflows/launcher.yml").read_text()
        step = workflow.split("      - id: decide\n", 1)[1].split("\n  build:", 1)[0]
        script = textwrap.dedent(step.split("        run: |\n", 1)[1])
        script = script.replace("${{ steps.mode.outputs.dry_run }}", "true")
        for status in (23, 0):
            with self.subTest(status=status), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                (root / "tools").mkdir()
                # Execute the workflow's real pipeline with a controlled decision subprocess.
                (root / "tools/release_train.py").write_text(
                    "import json, os, sys\n"
                    f"sys.exit({status})\n" if status else
                    "import json, os\n"
                    "print(json.dumps({'reason': 'ready', 'tag': 'launcher@0.2.0'}))\n"
                    "with open(os.environ['GITHUB_OUTPUT'], 'a') as out:\n"
                    "    out.write('tag=launcher@0.2.0\\n')\n")
                output = root / "output"
                output.touch()
                result = subprocess.run(["bash", "-e", "-c", script], cwd=root, capture_output=True,
                                        text=True, env={**os.environ, "GITHUB_OUTPUT": str(output),
                                        "GITHUB_STEP_SUMMARY": str(root / "summary"), "GITHUB_SHA": "head",
                                        "BASE": "", "EVENT": "workflow_dispatch", "DRY_RUN": "true",
                                        "SOURCE_SHA": "head", "FORCE": ""})
                if status:
                    self.assertEqual(result.returncode, status, result.stderr)
                    self.assertEqual(output.read_text(), "")
                    self.assertFalse((root / "summary").exists())
                else:
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertEqual(output.read_text(),
                                     "tag=launcher@0.2.0\nversion=0.2.0\nrelease=true\nhead=head\n")

    def test_player_facing_paths(self):
        exporters = {"mods/horizon/src/test/java/dev/horizonlab/parity/VanillaMobDataExporter.java"}
        self.assertTrue(train.player_facing("crates/horizon-client/src/game.rs", "game"))
        self.assertTrue(train.player_facing("Cargo.lock", "game"))
        self.assertTrue(train.player_facing("xtask/src/main.rs", "game"))
        self.assertTrue(train.player_facing(next(iter(exporters)), "game", exporters))
        self.assertFalse(train.player_facing("mods/horizon/src/test/java/dev/horizonlab/parity/VanillaFoodOracle.java", "game", exporters))
        self.assertFalse(train.player_facing("crates/horizon-world/examples/probe.rs", "game"))
        self.assertFalse(train.player_facing("crates/horizon-client/README.md", "game"))
        self.assertFalse(train.player_facing("tools/test_food_parity.py", "game"))
        self.assertFalse(train.player_facing("launcher/src/main.rs", "game"))
        self.assertTrue(train.player_facing("launcher/src/main.rs", "launcher"))
        self.assertTrue(train.player_facing("crates/horizon-prepare/src/lib.rs", "launcher"))
        self.assertFalse(train.player_facing("launcher/links.json", "launcher"))
        self.assertFalse(train.player_facing("crates/horizon-client/src/game.rs", "launcher"))

    def test_the_exporters_are_those_horizon_prepare_names(self):
        found = train.exporters()
        self.assertIn(train.EXPORTERS + "VanillaMobDataExporter.java", found)
        self.assertTrue(all(path.endswith(".java") for path in found))

    def test_game_tags_count_up_within_a_day(self):
        today = dt.date(2026, 10, 15)
        self.assertEqual(train.next_game_tag(["v2026.10.14", "launcher-v0.1.4"], today), "v2026.10.15")
        self.assertEqual(train.next_game_tag(["v2026.10.15"], today), "v2026.10.15.01")
        self.assertEqual(train.next_game_tag(["v2026.10.15", "v2026.10.15.01", "v2026.10.15.02"], today), "v2026.10.15.03")
        # Unpadded tags from before the change still count, and the next one sorts after them as text too.
        self.assertEqual(train.next_game_tag(["v2026.10.15", "v2026.10.15.9", "v2026.10.15.20"], today), "v2026.10.15.21")
        self.assertEqual(train.next_game_tag(["v2026.10.15", "v2026.10.15.08"], today), "v2026.10.15.09")
        tags = ["v2026.10.15.09", "v2026.10.15.10", "v2026.10.15"]
        self.assertEqual(sorted(tags), sorted(tags, key=train.tag_key))

    def test_launcher_tags_bump_the_patch_or_the_minor(self):
        tags = ["launcher-v0.1.3", "launcher-v0.1.10", "v2026.10.15", "launcher-v0.1.4"]
        self.assertEqual(train.next_launcher_tag(tags, minor=False), "launcher-v0.1.11")
        self.assertEqual(train.next_launcher_tag(tags, minor=True), "launcher-v0.2.0")

    def test_0_2_tags_follow_every_tag_from_before(self):
        order = ["launcher-v0.1.9", "launcher-v0.16.0", "v2026.10.19.15", "v2026.10.21", "v0.2.0-beta.1", "v0.2.0",
                 "v0.2.1-beta", "v0.2.1-beta.1", "v0.2.1-beta.2", "v0.2.1-beta.11", "v0.2.1-rc.1", "v0.2.1", "v0.2.9",
                 "v0.2.10", "v0.3.0-1", "v0.3.0-alpha", "v0.3.0", "v1.0.0"]
        self.assertEqual(sorted(reversed(order), key=train.tag_key), order)
        self.assertGreater(train.tag_key("launcher@0.2.0"), train.tag_key("launcher-v0.16.0"))
        self.assertEqual(train.tag_key("v2026.10.19"), train.tag_key("v2026.10.19.0"))
        for tag in ["v0.2.0", "v0.3.0-rc.1", "v2026.10.21", "v2026.10.19.15"]:
            self.assertTrue(train.is_game_tag(tag), tag)
        for tag in ["launcher@0.2.0", "launcher-v0.2.0", "v0.2", "vnext", "v2026.10.21-x", "v0.2.0-"]:
            self.assertFalse(train.is_game_tag(tag), tag)
        self.assertTrue(train.is_launcher_tag("launcher@0.2.0") and train.is_launcher_tag("launcher-v0.16.0"))
        self.assertFalse(train.is_launcher_tag("v0.2.0"))

    def test_tags_on_a_release_line(self):
        line = (0, 2)
        before = ["v2026.10.19.15", "v2026.10.21", "launcher-v0.16.0", "launcher-v0.2.0", "launcher-v0.2.1"]
        today = dt.date(2026, 10, 23)
        self.assertEqual(train.next_game_tag(before, today, line), "v0.2.0")
        self.assertEqual(train.next_launcher_tag(before, minor=True, line=line), "launcher@0.2.0")
        self.assertTrue(train.first_on_line(before, "v", line))
        after = before + ["v0.2.0", "v0.2.1", "launcher@0.2.0", "v0.3.0-rc.1"]
        self.assertEqual(train.next_game_tag(after, today, line), "v0.2.2")
        # A feature doesn't move the line; only .github/release-line.txt does.
        self.assertEqual(train.next_launcher_tag(after, minor=True, line=line), "launcher@0.2.1")
        self.assertFalse(train.first_on_line(after, "v", line))
        self.assertTrue(train.first_on_line(after, "v", (0, 3)), "a pre-release isn't the line's first release")
        self.assertEqual(train.next_game_tag(after, today, (0, 3)), "v0.3.0")
        with self.assertRaises(SystemExit):
            train.next_game_tag(after + ["v0.3.0"], today, line)
        # Without a line, the tags from before 0.2, whatever 0.2 tags exist.
        self.assertEqual(train.next_game_tag(before, today), "v2026.10.23")
        self.assertEqual(train.next_launcher_tag(after, minor=False), "launcher-v0.16.1")

    def test_the_release_line_file(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "release-line.txt"
            self.assertIsNone(train.release_line(path))
            path.write_text("0.2\n")
            self.assertEqual(train.release_line(path), (0, 2))
            path.write_text("0.2.0\n")
            with self.assertRaises(SystemExit):
                train.release_line(path)
        # Not set on main until the switch to 0.2 (docs/versioning.md).
        self.assertIsNone(train.release_line()) if not train.LINE_FILE.exists() else self.assertIsNotNone(train.release_line())

    def test_a_new_line_releases_at_once_unless_paused(self):
        self.assertEqual(train.decision("game", [], NOW, new_line="0.2"), (True, "the first release of 0.2"))
        self.assertFalse(train.decision("game", [], NOW, paused=True, new_line="0.2")[0])
        self.assertFalse(train.decision("game", [], NOW)[0])

    def test_the_first_release_of_0_2_says_what_changed(self):
        title, body = train.notes("game", "v0.2.0", [pr(1)], [], previous="v2026.10.21")
        self.assertTrue(title.startswith("Horizon 0.2.0: "), title)
        body = body.removeprefix(train.PLAYERS + "\n\n")
        self.assertTrue(body.startswith("Horizon is numbered from now on: this is Horizon 0.2.0, the release after 2026.10.21."))
        self.assertIn("`horizon-server update --version v0.2.0`", body)
        title, body = train.notes("launcher", "launcher@0.2.0", [], [], previous="launcher-v0.17.0")
        self.assertEqual(title, "Horizon launcher 0.2.0")
        self.assertIn("the release after launcher 0.17.0", body)
        self.assertIn("Rebuilt from the latest main.", body)
        body = train.notes("game", "v0.3.0", [pr(1)], [], previous="v0.2.14")[1]
        self.assertTrue(body.startswith(train.PLAYERS + "\n\nThe first release of Horizon 0.3."))
        for version, previous in [("v0.2.1", "v0.2.0"), ("v0.3.0-rc.1", "v0.2.9"), ("v2026.10.23", "v2026.10.21")]:
            body = train.notes("game", version, [pr(1)], [], previous=previous)[1]
            self.assertTrue(body.startswith(train.PLAYERS + "\n\n### Fixed"), (version, body))

    def test_decision(self):
        minute = dt.timedelta(minutes=1)
        decide = lambda prs, **kw: train.decision("game", prs, NOW, **kw)[0]
        hour = 60 * minute
        five = lambda newest: [pr(n, merged=newest + n * minute) for n in range(5)]
        self.assertTrue(decide(five(6 * minute)), "a batch, settled for the debounce")
        self.assertFalse(decide(five(2 * minute)), "a batch still merging")
        self.assertFalse(decide(five(6 * minute)[:4]), "not yet a batch")
        self.assertFalse(decide([pr(1, merged=5 * hour)]), "a lone fix waits for others")
        self.assertTrue(decide([pr(1, merged=6 * hour)]), "until the oldest has waited 6 hours")
        self.assertTrue(decide([pr(1, merged=7 * hour), pr(2, merged=minute)]), "merges that never pause still ship")
        self.assertFalse(decide([pr(n, merged=6 * minute) for n in range(4)] + [pr(9, labels=["skip-release"])]),
                         "work players don't see doesn't fill a batch")
        self.assertTrue(decide([pr(1, labels=["release-now"], merged=minute)]))
        self.assertFalse(decide([pr(1, paths=[])]), "nothing players run")
        self.assertFalse(decide([pr(1, labels=["skip-release"])]))
        self.assertFalse(decide([pr(1, labels=["release-now", "skip-release"])]))
        self.assertFalse(decide([pr(1)], paused=True))
        self.assertTrue(decide([], paused=True, force=True))

    def test_an_open_release_blocker_holds_every_release_even_forced(self):
        release, why = train.decision("game", [pr(1, labels=["release-now"])], NOW, blockers=[7, 9])
        self.assertFalse(release)
        self.assertEqual(why, "held by release-blocker #7, release-blocker #9")
        self.assertFalse(train.decision("game", [], NOW, force=True, blockers=[7])[0])
        self.assertFalse(train.decision("game", [], NOW, new_line="0.3", blockers=[7])[0])
        self.assertTrue(train.decision("game", [pr(1)], NOW, blockers=[])[0])

    @mock.patch.object(train, "REPO", "horizon-rust/horizon")
    def test_game_releases_ship_only_what_parity_main_passed(self):
        def runs(parity, checks):
            def gh_json(*arguments):
                table = parity if "parity-main" in arguments else checks
                if "--commit" in arguments:
                    sha = arguments[arguments.index("--commit") + 1]
                    return [{"status": s, "conclusion": c} for h, s, c in table if h == sha]
                if "--status" in arguments:
                    wanted = arguments[arguments.index("--status") + 1]
                    table = [(h, s, c) for h, s, c in table if c == wanted]
                return [{"headSha": h, "status": s, "conclusion": c} for h, s, c in table]
            return mock.patch.object(train, "gh_json", gh_json)
        checks = [("c", "in_progress", ""), ("b", "completed", "success"), ("a", "completed", "success")]
        with runs([("c", "queued", ""), ("b", "completed", "cancelled"), ("a", "completed", "success")], checks):
            self.assertEqual(train.green_main_head(), "b", "checks alone: the newest green checks")
            self.assertEqual(train.green_main_head(parity=True), "a", "a superseded parity-main run counts for nothing")
            self.assertTrue(train.passed_on("parity-main", "a"))
            self.assertFalse(train.passed_on("parity-main", "b"))
        with runs([("b", "completed", "failure"), ("a", "completed", "success")], checks):
            self.assertEqual(train.green_main_head(parity=True), "a",
                             "a later failed batch holds only its own merges, not the commit before it")
        with runs([("c", "completed", "success")], checks):
            self.assertIsNone(train.green_main_head(parity=True), "its checks must pass too")
        with runs([("c", "completed", "success"), ("a", "completed", "success")], checks):
            self.assertEqual(train.green_main_head(parity=True), "a", "the newest green commit whose checks passed")
        with runs([("c", "completed", "failure"), ("b", "completed", "cancelled")], checks):
            self.assertIsNone(train.green_main_head(parity=True), "nothing parity-main passed")
        with runs([("c", "completed", "success")], [("c", "completed", "success")]):
            self.assertEqual(train.green_main_head(), "c")
        with runs([], [("c", "completed", "failure"), ("b", "completed", "success")]):
            self.assertIsNone(train.green_main_head(), "checks alone: main's newest finished checks must pass")

    def test_launcher_manual_base_is_pinned_and_must_belong_to_main(self):
        workflow = (train.ROOT / ".github/workflows/launcher.yml").read_text()
        selection = workflow.split("          set -euo pipefail\n", 1)[1].split(
            "          python3 tools/release_train.py decide", 1)[0]
        script = "set -euo pipefail\n" + textwrap.dedent(selection) + '\nprintf "%s" "$HEAD_SHA"'
        with tempfile.TemporaryDirectory() as folder:
            def git(*args):
                return subprocess.check_output(["git", *args], cwd=folder, text=True,
                                               stderr=subprocess.DEVNULL).strip()
            git("init", "-b", "main")
            git("config", "user.name", "Test")
            git("config", "user.email", "test@example.invalid")
            git("commit", "--allow-empty", "-m", "main")
            main = git("rev-parse", "HEAD")
            git("remote", "add", "origin", folder)
            git("checkout", "-b", "topic")
            git("commit", "--allow-empty", "-m", "not main")
            topic = git("rev-parse", "HEAD")
            def select(base="", event="workflow_dispatch", dry_run="false"):
                return subprocess.run(["bash", "-c", script], cwd=folder, text=True,
                                      capture_output=True, env={**os.environ, "BASE": base,
                                      "EVENT": event, "DRY_RUN": dry_run, "SOURCE_SHA": topic})
            for dry_run in ("false", "true"):
                result = select(main, dry_run=dry_run)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stdout, main)
            for invalid in ("does-not-exist", topic, "--help"):
                result = select(invalid)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("::error::base", result.stdout)
            self.assertEqual(select().stdout, "", "default real manual run uses the gate picker")
            self.assertEqual(select(event="push").stdout, topic)
            self.assertEqual(select(dry_run="true").stdout, topic)

    @mock.patch.object(train, "REPO", "arismoko/horizon")
    def test_fork_base_requires_same_sha_main_push_gate_jobs(self):
        def record(sha, ident, event="push", branch="main"):
            return dict(head_sha=sha, id=ident, event=event, head_branch=branch,
                        status="completed", conclusion="failure")
        def job(name, status="completed", conclusion="success"):
            return dict(name=name, status=status, conclusion=conclusion)
        gates = [job(name) for name in ("verify", "launcher")]
        records = [record("pr", 1, "pull_request"), record("topic", 2, branch="topic"),
                   record("new", 3), record("good", 4), record("old", 5)]
        jobs = {3: gates[:-1], 4: gates + [job("client-data", conclusion="failure")]}
        def api(*arguments):
            self.assertEqual(arguments[:4], ("api", "--paginate", "--slurp", arguments[3]))
            path = arguments[3]
            self.assertNotIn("wasm-mods.yml", path, "the fixture runs in the daily run, not on main pushes")
            if "/ci.yml/runs?" in path:
                self.assertIn("branch=main&event=push", path)
                return [{"workflow_runs": records[:3]}, {"workflow_runs": records[3:]}]
            ident = int(path.split("/runs/")[1].split("/")[0])
            self.assertIn("filter=latest", path)
            return [{"jobs": jobs[ident][:1]}, {"jobs": jobs[ident][1:]}]
        with mock.patch.object(train, "REPO", "arismoko/horizon"), \
                mock.patch.object(train, "gh_json", side_effect=api) as requests:
            self.assertEqual(train.green_main_head(parity=True), "good", "missing gate is not success")
            self.assertEqual(train.green_main_head(), "good", "non-gate failure must not reject a build")
            jobs[3] = gates
            self.assertEqual(train.green_main_head(), "new")
            requests.reset_mock()
            self.assertEqual(train.green_main_head(eligible=lambda sha: sha == "good"), "good")
            paths = [call.args[3] for call in requests.call_args_list]
            self.assertFalse(any("/runs/3/jobs" in path for path in paths))
            self.assertTrue(any("/runs/4/jobs" in path for path in paths))
            for name in ("verify", "launcher"):
                for status, conclusion in [("queued", None), ("in_progress", "success"),
                                           ("completed", "skipped"), ("completed", "cancelled"),
                                           ("completed", "failure"), ("completed", "timed_out")]:
                    with self.subTest(gate=name, status=status, conclusion=conclusion):
                        jobs[3] = [entry for entry in gates if entry["name"] != name] + [job(name, status, conclusion)]
                        self.assertEqual(train.green_main_head(), "good")
            records.insert(3, record("new", 99))
            jobs[99] = gates
            self.assertEqual(train.green_main_head(), "good", "an older run cannot override the latest run")
            records[:] = records[:2]
            self.assertIsNone(train.green_main_head(), "PR checks cannot supply a release base")

    def test_the_daily_nightly_skips_only_the_batch_rules(self):
        fresh = [{"number": 7, "labels": [], "paths": ["crates/horizon-world/src/lib.rs"],
                  "merged_at": NOW - dt.timedelta(minutes=1)}]
        self.assertFalse(train.decision("game", fresh, NOW)[0], "the batch train waits for more")
        release, reason = train.decision("game", fresh, NOW, daily=True)
        self.assertTrue(release, "one fresh player-facing pull request publishes from a green daily")
        self.assertIn("daily run", reason)
        self.assertEqual(train.decision("game", fresh, NOW, paused=True, daily=True),
                         (False, "RELEASES_PAUSED is set"))
        self.assertEqual(train.decision("game", [], NOW, daily=True),
                         (False, "no merged pull request changed what players run"),
                         "the commit the published nightly was built on: nothing to publish")
        self.assertFalse(train.decision("game", fresh, NOW, blockers=[4], daily=True)[0])

    def test_fork_channels_check_daily_green_or_the_gate(self):
        with tempfile.TemporaryDirectory() as folder:
            def git(*args):
                return subprocess.check_output(["git", *args], cwd=folder, text=True, stderr=subprocess.DEVNULL).strip()
            git("init", "-q", "-b", "main")
            git("config", "user.name", "Test")
            git("config", "user.email", "test@example.invalid")
            commits = []
            for n in range(3):
                git("commit", "-q", "--allow-empty", "-m", str(n))
                commits.append(git("rev-parse", "HEAD"))
            git("tag", "daily-green", commits[1])
            with mock.patch.object(train, "ROOT", Path(folder)):
                self.assertEqual(train.channel_head("nightly", ""), commits[1], "the default is daily-green itself")
                self.assertEqual(train.channel_head("nightly", "daily-green"), commits[1])
                for other in (commits[0], commits[2]):
                    with self.assertRaisesRegex(SystemExit, "is not the commit `daily-green` points to"):
                        train.channel_head("nightly", other)
                git("tag", "-d", "daily-green")
                with self.assertRaisesRegex(SystemExit, "no `daily-green` tag yet"):
                    train.channel_head("nightly", "")
                # A snapshot: main's head by default, or any commit of main, with the gate green on it.
                git("update-ref", "refs/remotes/origin/main", commits[1])
                head = commits[1]
                green = {"workflow_runs": [dict(head_sha=head, head_branch="main", event="push", id=7)]}
                done = dict(status="completed", conclusion="success")
                gates = [{"jobs": [dict(name=name, **done) for name in ("verify", "launcher")]}]
                with mock.patch.object(train, "REPO", "arismoko/horizon"), \
                        mock.patch.object(train, "gh_json", side_effect=[[green], gates]):
                    self.assertEqual(train.channel_head("snapshot", ""), head, "a snapshot needs the gate only")
                with mock.patch.object(train, "REPO", "arismoko/horizon"), \
                        mock.patch.object(train, "gh_json", side_effect=[[green], [{"jobs": gates[0]["jobs"][:1]}]]):
                    with self.assertRaisesRegex(SystemExit, "a snapshot needs the gate green"):
                        train.channel_head("snapshot", head)
                pending = [{"jobs": gates[0]["jobs"][:1] + [dict(name="launcher", status="in_progress", conclusion=None)]}]
                with mock.patch.object(train, "REPO", "arismoko/horizon"), \
                        mock.patch.object(train, "gh_json", side_effect=[[green], pending]):
                    with self.assertRaisesRegex(SystemExit, "a snapshot needs the gate green"):
                        train.channel_head("snapshot", head)
                # Not on main (newer than main's head here), or not a commit: refused before any gate lookup.
                with mock.patch.object(train, "gh_json", side_effect=AssertionError("no lookup")):
                    with self.assertRaisesRegex(SystemExit, "is not on main"):
                        train.channel_head("snapshot", commits[2])
                    with self.assertRaisesRegex(SystemExit, "not a commit of this repository"):
                        train.channel_head("snapshot", "f" * 40)

    def test_a_snapshot_is_tagged_by_kind_date_and_commit(self):
        sha = "0c02067" + "a" * 33
        self.assertEqual(train.build_tag("snapshot", "2026-11-13", sha), "snapshot-2026-11-13-0c02067")
        self.assertEqual(train.build_tag("nightly", "2026-11-13", sha), "nightly-2026-11-13-0c02067")

    def test_a_release_is_published_only_after_its_smoke_test_and_with_no_blocker(self):
        drew = {"error": None, "frames": 12710, "chunks": 81, "meshed_sections": 550, "player": [160.5, 71.0, 156.5]}
        self.assertIsNone(train.gate(drew, []))
        self.assertEqual(train.gate(drew, [4]), "release-blocker #4 is open")
        self.assertEqual(train.gate(None, []), "no smoke-test.json: the release smoke test did not run")
        self.assertEqual(train.gate(None, [4]),
                         "no smoke-test.json: the release smoke test did not run; release-blocker #4 is open")
        self.assertEqual(train.smoke_problem({**drew, "error": "device lost"}), "the smoke run failed: device lost")
        for broken in ({**drew, "frames": 1499}, {**drew, "chunks": 0}, {**drew, "meshed_sections": 0},
                       {**drew, "player": None}, {}):
            self.assertEqual(train.smoke_problem(broken), "the smoke run did not draw a world", broken)

    def test_notes_never_call_a_release_alpha_or_beta(self):
        _, body = train.notes("game", "v0.2.12", [pr(1)], [])
        self.assertNotIn("alpha", body.lower())
        self.assertNotIn("beta", body.lower())
        self.assertNotIn("Stable", body)

    def test_notes(self):
        prs = [pr(5, "Nether portals: vanilla timing and linking (fixes #94)", issues=[94]),
               pr(6, "Controllers: gilrs input layer and gameplay", labels=["feature"], author="cosmic"),
               pr(7, "Release folder in Rust", labels=["skip-release"]),
               pr(8, "Website footer", paths=[])]
        title, body = train.notes("game", "v2026.10.15.1", prs, ["- Sound cuts out after an hour."])
        self.assertEqual(title, "Horizon 2026.10.15.1: Controllers, nether portals")
        self.assertIn("### New\n- Controllers: gilrs input layer and gameplay (#6)", body)
        self.assertIn("### Fixed\n- Nether portals: vanilla timing and linking (#5, fixes #94)", body)
        self.assertIn("### Known issues\n- Sound cuts out after an hour.", body)
        self.assertNotIn("#7", body)
        self.assertNotIn("#8", body)
        self.assertIn("### Contributors\nThanks to @cosmic.", body)
        self.assertTrue(body.endswith("---\n" + train.DISCLAIMER + "\n"), body)
        self.assertLess(body.index("### New"), body.index("### Fixed"))
        title, _ = train.notes("game", "v2026.10.15.2", [pr(9, "Horizon's mark", labels=["feature"]),
                                                         pr(10, "Game menus", labels=["feature"])], [])
        self.assertEqual(title, "Horizon 2026.10.15.2: Game menus, Horizon's mark", "proper nouns keep their capital")

    def test_a_long_release_is_named_after_its_first_titles(self):
        prs = [pr(n, f"Change number {n} with a fairly long title") for n in range(1, 10)]
        title, _ = train.notes("game", "v2026.10.15", prs, [])
        self.assertTrue(title.startswith("Horizon 2026.10.15: Change number 9"), title)
        self.assertTrue(title.endswith("more fixes"), title)
        self.assertLess(len(title), 100)

    def test_a_feature_split_over_several_pull_requests_is_named_once(self):
        prs = [pr(1, "Controllers: rumble", labels=["feature"]), pr(2, "Controllers: menus", labels=["feature"]),
               pr(3, "Maces: smash attacks"), pr(4, "Fix stuck keys when opening Options")]
        title, _ = train.notes("game", "v2026.10.15", prs, [])
        self.assertEqual(title, "Horizon 2026.10.15: Controllers, stuck keys when opening Options, maces")
        self.assertEqual(train.summary([], [pr(n, f"Change number {n} with a fairly long title") for n in range(4)]),
                         "Change number 3 with a fairly long title and 3 more fixes")

    def test_launcher_notes(self):
        title, body = train.notes("launcher", "launcher-v0.2.0", [pr(3, "Launcher: say the self-update succeeded")], [])
        self.assertEqual(title, "Horizon launcher 0.2.0")
        self.assertIn("### Fixed\n- Launcher: say the self-update succeeded (#3)", body)
        self.assertNotIn("Known issues", body)
        self.assertNotIn(train.PLAYERS, body)
        self.assertNotIn(train.PLAYERS_FOOTER, body)
        self.assertIn("Your launcher updates itself.", body)

    def test_a_game_release_opens_with_the_way_to_the_launcher(self):
        # GitHub's latest is a game release: whoever lands on it is sent to the launcher first (#1647).
        _, body = train.notes("game", "v0.2.50", [pr(1)], ["- Sound cuts out after an hour."])
        self.assertTrue(body.startswith(train.PLAYERS + "\n\n"))
        self.assertTrue(body.endswith("\n\n" + train.PLAYERS_FOOTER + "\n\n---\n" + train.DISCLAIMER + "\n"), body)
        self.assertNotIn("\n", train.PLAYERS, "one line: one HTML block, which ends at the blank line after it")
        self.assertEqual(body.count("playhorizon.dev"), 2, "said at the top and the bottom")
        _, body = train.notes("game", "v0.2.51", [], [])
        self.assertTrue(body.startswith(train.PLAYERS + "\n\nRebuilt from the latest main."))

    def test_known_issues_are_the_bullets_of_the_file(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp, "known-issues.md")
            path.write_text("# Known issues\n\nSome text.\n\n- One.\n- Two, with **bold**.  \n"
                            "  continued with `code`\n\tand a [link](https://example.com).  \n- Three.\n")
            self.assertEqual(train.known_issues(path),
                             ["- One.", "- Two, with **bold**. continued with `code` and a [link](https://example.com).",
                              "- Three."])
            self.assertEqual(train.known_issues(Path(temp, "missing.md")), [])

    def test_known_issues_do_not_include_unrelated_text(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp, "known-issues.md")
            for separator in ["\n", "  \n", "# Heading\n", "Some text.\n"]:
                with self.subTest(separator=separator):
                    path.write_text("  Intro.\n- One.\n  continued.\n" + separator + "  Not an issue.\n- Two.\n")
                    self.assertEqual(train.known_issues(path), ["- One. continued.", "- Two."])
            path.write_text("# Known issues\n\nNo issues.\n")
            self.assertEqual(train.known_issues(path), [])
            path.write_text("")
            self.assertEqual(train.known_issues(path), [])

    def test_notes_keep_the_wrapped_shaderpacks_issue(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp, "known-issues.md")
            path.write_text("- Shader packs have been removed for now. Packs in your\n"
                            "  `shaderpacks` folder are left where they are.\n")
            _, body = train.notes("game", "v0.2.70", [pr(1)], train.known_issues(path))
            self.assertIn("### Known issues\n- Shader packs have been removed for now. Packs in your "
                          "`shaderpacks` folder are left where they are.\n", body)




class NightlyTrainTests(unittest.TestCase):
    """Since 2026-10-28 the train decides when a Nightly goes out (release.yml starts nightly-build.yml)."""

    PRS = [{"number": 1, "labels": [], "paths": ["crates/x.rs"]}]

    def test_one_build_at_a_time_and_twelve_a_day(self):
        self.assertEqual(train.nightly_decision(True, "5 pull requests", self.PRS, None, 3)[:2], (True, "5 pull requests"))
        release, reason, _ = train.nightly_decision(True, "5", self.PRS, 4242, 3)
        self.assertFalse(release)
        self.assertIn("run 4242", reason)
        release, reason, urgent = train.nightly_decision(True, "5", self.PRS, None, 12)
        self.assertFalse(release)
        self.assertFalse(urgent)
        self.assertIn("12 Nightlies today", reason)
        self.assertEqual(train.nightly_decision(False, "waiting", self.PRS, None, 0)[:2], (False, "waiting"))

    def test_release_now_and_force_pass_the_cap_but_not_a_running_build(self):
        urgent = [{"number": 2, "labels": ["release-now"], "paths": ["crates/x.rs"]}]
        self.assertEqual(train.nightly_decision(True, "release-now: #2", urgent, None, 12), (True, "release-now: #2", True))
        self.assertTrue(train.nightly_decision(True, "forced", self.PRS, None, 30, force=True)[0])
        self.assertFalse(train.nightly_decision(True, "forced", self.PRS, 7, 0, force=True)[0])

    def test_a_snapshot_build_never_holds_the_nightly_back(self):
        def runs(*entries):
            return [dict(databaseId=n, status=status, displayTitle=title) for n, (status, title) in enumerate(entries, 1)]
        snapshot = ("in_progress", "snapshot build " + "a" * 40)
        with mock.patch.object(train, "gh_json", return_value=runs(snapshot, ("queued", "snapshot build "))):
            self.assertIsNone(train.nightly_building(), "an active snapshot, real or dry, is no Nightly")
        with mock.patch.object(train, "gh_json", return_value=runs(snapshot, ("queued", "nightly build"))):
            self.assertEqual(train.nightly_building(), 2)
        with mock.patch.object(train, "gh_json", return_value=runs(("in_progress", "nightly build (request 7)"))):
            self.assertEqual(train.nightly_building(), 1)
        with mock.patch.object(train, "gh_json", return_value=runs(("completed", "nightly build"), snapshot)):
            self.assertIsNone(train.nightly_building())
        # The workflow names a snapshot's run exactly so, and a Nightly's never so.
        run_name = (train.ROOT / ".github/workflows/nightly-build.yml").read_text().split("\nrun-name: ", 1)[1].split("\n", 1)[0]
        self.assertIn("inputs.kind == 'snapshot' && format('snapshot build {0}', inputs.base)", run_name)
        self.assertIn("format('nightly build{0}'", run_name)

    def test_a_running_nightly_never_holds_a_snapshot_back(self):
        sha = "a" * 40
        argv = ["release_train.py", "decide", "--kind", "game", "--channel", "snapshot"]
        with mock.patch("sys.argv", argv), mock.patch.object(train, "channel_head", return_value=sha), \
                mock.patch.object(train, "run", return_value=sha + "\n"), \
                mock.patch.object(train, "nightly_building", side_effect=AssertionError("asked")), \
                mock.patch.object(train, "write_outputs") as outputs, mock.patch("sys.stdout"):
            self.assertEqual(train.main(), 0)
        values = outputs.call_args.args[0]
        self.assertEqual(values["release"], "true")
        self.assertTrue(values["tag"].startswith("snapshot-") and values["tag"].endswith("-aaaaaaa"))

    def test_the_cap_is_the_nightly_sets(self):
        from tools.ci import nightly_set
        self.assertEqual(train.NIGHTLY_CAP, nightly_set.MAX_BUILDS_PER_DAY)


if __name__ == "__main__":
    unittest.main()
