"""Snapshots (#4022): release.yml kind snapshot builds a commit of main with the gate green and publishes it under its
own tag, never as the Nightly and never moving `daily-green`."""
import datetime as dt
import os
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from tests.test_ci_concurrency import ROOT, yaml
from tools.ci import nightly

WORKFLOWS = ROOT / ".github/workflows"


def load(name):
    return yaml.load((WORKFLOWS / name).read_text(), Loader=yaml.BaseLoader)


class ReleaseSnapshotTests(unittest.TestCase):
    def setUp(self):
        self.workflow = load("release.yml")
        self.jobs = self.workflow["jobs"]
        self.snapshot = {name: job for name, job in self.jobs.items() if name.startswith("snapshot")}

    def test_only_main_without_dry_run_publishes(self):
        mode = self.jobs["decide"]["steps"][1]
        self.assertEqual(mode["run"],
                         "echo \"dry_run=${{ inputs.dry_run || github.ref != 'refs/heads/main' }}\" >> \"$GITHUB_OUTPUT\"")
        self.assertEqual(self.workflow["on"]["workflow_dispatch"]["inputs"]["dry_run"]["default"], "true")
        publish = self.snapshot["snapshot-sdk-publish"]
        self.assertEqual(publish["with"]["publish"], "${{ needs.decide.outputs.dry_run != 'true' }}")
        self.assertEqual(publish["uses"], "./.github/workflows/sdk-publish.yml")
        # The game is built by `nightly build`, which publishes only when its own dry_run is false: it gets ours.
        run = self.snapshot["snapshot"]["steps"][0]["run"]
        self.assertIn('-f dry_run="$DRY_RUN"', run)
        self.assertEqual(self.snapshot["snapshot"]["steps"][0]["env"]["DRY_RUN"], "${{ needs.decide.outputs.dry_run }}")

    def test_a_snapshot_never_moves_daily_green_or_writes_the_nightly(self):
        text = yaml.safe_dump(self.snapshot)
        self.assertNotIn("daily-green", text)
        self.assertNotIn("git tag", text)
        self.assertNotIn("gh release", text)
        run = self.snapshot["snapshot"]["steps"][0]["run"]
        self.assertIn("gh workflow run nightly-build.yml --repo \"$GITHUB_REPOSITORY\" --ref main -f kind=snapshot", run)
        for name, job in self.snapshot.items():
            self.assertIn("inputs.kind == 'snapshot'", job["if"], name)
        self.assertIn("inputs.kind == 'nightly'", self.jobs["nightly"]["if"])

    def test_the_sdk_is_built_and_published_in_this_run_before_the_game(self):
        self.assertEqual(self.snapshot["snapshot-sdk"]["uses"], "./.github/workflows/prebuilt-game-build.yml")
        self.assertEqual(self.snapshot["snapshot-sdk"]["with"], {"commit": "${{ needs.decide.outputs.head }}"})
        self.assertEqual(self.snapshot["snapshot-sdk-publish"]["with"],
                         {"commit": "${{ needs.decide.outputs.head }}",
                          "publish": "${{ needs.decide.outputs.dry_run != 'true' }}"})
        game = self.snapshot["snapshot"]
        self.assertEqual(game["needs"], ["decide", "snapshot-sdk", "snapshot-sdk-publish"])
        self.assertIn("needs.snapshot-sdk.result == 'success'", game["if"])
        self.assertIn("(needs.snapshot-sdk-publish.result == 'success' || needs.snapshot-sdk-publish.result == 'skipped')",
                      game["if"].replace("\n", " "))

    def test_snapshots_never_queue_with_nightlies(self):
        self.assertIn("inputs.kind == 'snapshot' && '-snapshot'", self.workflow["concurrency"]["group"])


class NightlyBuildSnapshotTests(unittest.TestCase):
    def setUp(self):
        self.workflow = load("nightly-build.yml")
        self.steps = {step.get("name"): step for step in self.workflow["jobs"]["publish"]["steps"]}

    def test_a_snapshot_publishes_only_its_own_tag(self):
        nightly_step = self.steps["Publish as the `nightly` pre-release"]
        snapshot_step = self.steps["Publish as the snapshot-<date>-<short> pre-release"]
        self.assertIn("inputs.kind == 'nightly'", nightly_step["if"])
        self.assertIn("inputs.kind == 'snapshot'", snapshot_step["if"])
        self.assertIn("needs.pick.outputs.dry_run != 'true'", snapshot_step["if"])
        run = snapshot_step["run"]
        self.assertIn('gh release create "$TAG"', run)
        for never in ("rotate", "keep-source", "daily-green", "release create nightly", "prune --days", "release edit"):
            self.assertNotIn(never, run)
        self.assertIn('--title "Snapshot $DATE · $SHORT"', run)
        self.assertLess(run.index("require-sdk"), run.index("gh release create"))

    def test_the_channel_signed_and_checked_is_the_kind(self):
        sign = self.workflow["jobs"]["sign"]["steps"][2]
        self.assertEqual(sign["env"]["KIND"], "${{ inputs.kind }}")
        self.assertEqual(sign["env"]["ALSO_FOR"],
                         "${{ inputs.kind }}-${{ needs.pick.outputs.date }}-${{ needs.pick.outputs.short }}")
        verify = self.steps["The signatures, checked as launchers check them"]["run"]
        self.assertIn('--tag "$KIND" --publishing', verify)
        self.assertIn('--tag "${KIND}-${DATE}-${SHORT}"', verify)
        self.assertEqual(self.workflow["on"]["workflow_dispatch"]["inputs"]["kind"]["default"], "nightly")

    def test_snapshot_optional_platforms_keep_nightly_and_release_requirements(self):
        game = load("game-build.yml")
        build = game["jobs"]["build"]
        expression = build["continue-on-error"].removeprefix("${{").removesuffix("}}").strip()
        self.assertEqual(game["on"]["workflow_call"]["inputs"]["kind"]["default"], "release")
        self.assertEqual(self.workflow["jobs"]["build"]["with"]["kind"], "${{ inputs.kind }}")
        for kind in ("snapshot", "nightly", "release"):
            optional = set()
            for row in build["strategy"]["matrix"]["include"]:
                condition = expression.replace("matrix.name", repr(row["name"])).replace("inputs.kind", repr(kind))
                if eval(condition.replace("||", "or").replace("&&", "and"), {"__builtins__": {}}):
                    optional.add(row["name"])
            self.assertEqual(optional, {"windows-arm64", "linux-musl"} | ({"macos-x64"} if kind == "snapshot" else set()))
        self.assertEqual(game["jobs"]["linux-smoke"]["needs"], ["runner-configuration", "build"])
        self.assertEqual(self.workflow["jobs"]["sign"]["needs"], ["pick", "build"])
        self.assertEqual(self.workflow["jobs"]["publish"]["needs"], ["pick", "build", "sign"])

    @unittest.skipIf(os.name == "nt", "executes the POSIX signing step")
    def test_missing_intel_mac_is_signed_only_for_snapshot_and_notes_name_missing_assets(self):
        sign = self.workflow["jobs"]["sign"]["steps"][2]["run"]
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root / "dist").mkdir()
            (root / "tools/ci").mkdir(parents=True)
            (root / "tools/ci/sign_release.sh").write_text(
                'for file in "${@:2}"; do test -f "$file" || exit 9; done\nprintf "%s\\n" "$@" > signed\n')
            for name in ("horizon-linux-x86_64.tar.gz", "horizon-windows-x64.zip", "horizon-macos-arm64.zip",
                         "horizon-server-linux-x86_64.tar.gz", "horizon-source.tar.gz"):
                (root / "dist" / name).touch()
            for kind, code in (("snapshot", 0), ("nightly", 9)):
                result = subprocess.run(["bash", "-e", "-o", "pipefail", "-c", sign], cwd=root,
                                        env=dict(os.environ, KIND=kind, GITHUB_REPOSITORY="example/test"),
                                        capture_output=True, text=True)
                self.assertEqual(result.returncode, code, result.stdout + result.stderr)
            published = self.steps["Publish as the snapshot-<date>-<short> pre-release"]["run"]
            notes = published[published.index("# Archive presence"):].split("# Exactly the files", 1)[0]
            subprocess.run(["bash", "-e", "-o", "pipefail", "-c", notes], cwd=root, check=True)
            body = (root / "dist/nightly-notes.md").read_text()
            self.assertIn("Built: `horizon-macos-arm64.zip`", body)
            for missing in ("horizon-macos-x64.zip", "horizon-windows-arm64.zip", "horizon-linux-musl-x86_64.tar.gz"):
                self.assertIn(f"Failed or unavailable; not published: `{missing}`", body)

    def test_snapshots_never_replace_a_waiting_nightly(self):
        group = self.workflow["concurrency"]["group"]
        self.assertIn("'nightly-build'", group)
        self.assertIn("format('snapshot-build{0}'", group)


class SnapshotToolsTests(unittest.TestCase):
    def test_old_snapshots_go_but_the_newest_and_everything_else_stay(self):
        tags = [{"tagName": "snapshot-2026-11-01-aaaaaaa", "isPrerelease": True},
                {"tagName": "snapshot-2026-11-02-bbbbbbb", "isPrerelease": True},
                {"tagName": "snapshot-2026-11-12-ccccccc", "isPrerelease": True},
                {"tagName": "prebuilt-" + "d" * 40, "isPrerelease": True},
                {"tagName": "sdk-" + "d" * 40, "isPrerelease": True},
                {"tagName": "nightly-2026-11-01", "isPrerelease": True},
                {"tagName": "snapshot-2026-11-01", "isPrerelease": True},
                {"tagName": "snapshot-2026-11-01-eeeeeee", "isPrerelease": False}]
        today = dt.date(2026, 11, 13)
        self.assertEqual(nightly.stale_snapshots(tags, today, 7),
                         ["snapshot-2026-11-01-aaaaaaa", "snapshot-2026-11-02-bbbbbbb"])
        # A long quiet spell: the newest stays however old.
        self.assertEqual(nightly.stale_snapshots(tags[:2], dt.date(2027, 1, 1), 7), ["snapshot-2026-11-01-aaaaaaa"])

    def test_pick_checks_the_gate_and_names_the_snapshot(self):
        sha = "0c02067" + "a" * 33
        with tempfile.TemporaryDirectory() as folder:
            out = Path(folder) / "out"
            back = os.getcwd()
            os.chdir(folder)
            try:
                env = {"GITHUB_REPOSITORY": "arismoko/horizon", "GITHUB_OUTPUT": str(out), "NIGHTLY_BASE": sha}
                with mock.patch.dict(os.environ, env), mock.patch.object(nightly, "git"), \
                        mock.patch.object(nightly.train, "channel_head", return_value=sha) as gate, \
                        mock.patch.object(nightly, "sdk_ready", return_value=True), \
                        mock.patch.object(nightly, "gh", return_value=""):
                    nightly.pick_snapshot()
                gate.assert_called_once_with("snapshot", sha)
                values = dict(line.split("=", 1) for line in out.read_text().splitlines())
                today = dt.datetime.now(dt.timezone.utc).date().isoformat()
                self.assertEqual(values["tag"], f"snapshot-{today}-0c02067")
                self.assertEqual(values["version"], f"snapshot-{today}")
                self.assertEqual(values["build"], "true")
                self.assertIn(f"horizon-launcher install snapshot-{today}-0c02067", Path("nightly-notes.md").read_text())
                # Published already (the same commit the same day): skipped, never replaced.
                with mock.patch.dict(os.environ, env), mock.patch.object(nightly, "git"), \
                        mock.patch.object(nightly.train, "channel_head", return_value=sha), \
                        mock.patch.object(nightly, "sdk_ready", return_value=True), \
                        mock.patch.object(nightly, "gh", return_value='{"tagName": "x"}'):
                    nightly.pick_snapshot()
                self.assertIn("build=false", out.read_text().splitlines()[-10:])
                self.assertIn("sdk=true", out.read_text().splitlines()[-10:])
                # No published SDK: a dry run builds without it (sdk=false: no sdk.json), a real snapshot stops.
                with mock.patch.dict(os.environ, {**env, "DRY_RUN": "true"}), mock.patch.object(nightly, "git"), \
                        mock.patch.object(nightly.train, "channel_head", return_value=sha), \
                        mock.patch.object(nightly, "sdk_ready", return_value=False):
                    nightly.pick_snapshot()
                self.assertIn("sdk=false", out.read_text().splitlines()[-10:])
                # No published SDK: a real snapshot stops.
                with mock.patch.dict(os.environ, env), mock.patch.object(nightly, "git"), \
                        mock.patch.object(nightly.train, "channel_head", return_value=sha), \
                        mock.patch.object(nightly, "sdk_ready", return_value=False):
                    with self.assertRaises(SystemExit):
                        nightly.pick_snapshot()
            finally:
                os.chdir(back)


if __name__ == "__main__":
    unittest.main()
