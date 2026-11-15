"""Dry picks must never become release bookkeeping, including after failure."""
import contextlib
import datetime as dt
import json
import os
from pathlib import Path
import subprocess
import tempfile
import textwrap
import unittest
from unittest import mock

from tools.ci import nightly

ROOT = Path(__file__).resolve().parents[1]


class DryStateTests(unittest.TestCase):
    def test_only_real_builds_write_state_and_consume_the_daily_quota(self):
        for dry in (True, False):
            with self.subTest(dry=dry), tempfile.TemporaryDirectory() as folder, contextlib.chdir(folder), \
                    mock.patch.dict(os.environ, {"GITHUB_RUN_ID": "123", "GITHUB_RUN_ATTEMPT": "2"}, clear=True), \
                    mock.patch.object(nightly.ns, "fetch_heads", return_value=[]), \
                    mock.patch.object(nightly.ns, "assemble", return_value="a" * 40), \
                    mock.patch.object(nightly.ns, "builds_today", return_value=[]) as quota, \
                    mock.patch.object(nightly.ns, "push", return_value=True) as push, \
                    mock.patch.object(nightly.ns, "update_state") as update, \
                    mock.patch.object(nightly, "git", return_value=""), \
                    mock.patch.object(nightly, "merged_since", return_value=[]), \
                    mock.patch.object(nightly.train, "known_issues", return_value=[]), \
                    mock.patch.object(nightly, "output") as output:
                nightly.build_set(nightly.Ref("main", "a" * 40, dt.datetime.now(dt.timezone.utc)),
                                  [], [], "2026-11-10", True, dry)
                expected = "nightly/dry-run-123-2" if dry else "nightly/build-2026-11-10-1"
                self.assertEqual(push.call_args.args[:2], ("a" * 40, expected))
                self.assertEqual(output.call_args.kwargs["source"], expected)
                if dry:
                    quota.assert_not_called()
                    update.assert_not_called()
                else:
                    quota.assert_called_once_with("2026-11-10")
                    update.assert_called_once()
                    _, history, _ = update.call_args.args[0]({}, "")
                    self.assertEqual(json.loads(history)["ref"], expected)
                    self.assertFalse(json.loads(history)["dry_run"])

    def test_dry_pick_cannot_push_without_an_unambiguous_cleanup_identity(self):
        with mock.patch.dict(os.environ, {}, clear=True), \
                mock.patch.object(nightly.ns, "fetch_heads", return_value=[]), \
                mock.patch.object(nightly.ns, "assemble", return_value="a" * 40), \
                mock.patch.object(nightly, "git", return_value=""), \
                mock.patch.object(nightly.ns, "push") as push, \
                mock.patch.object(nightly.ns, "update_state") as update:
            with self.assertRaises(SystemExit):
                nightly.build_set(nightly.Ref("main", "a" * 40, None), [], [], "2026-11-10", True, True)
            push.assert_not_called()
            update.assert_not_called()

    def test_dry_schedule_never_refreshes_the_saved_selection(self):
        with mock.patch.dict(os.environ, {"DRY_RUN": "true", "GITHUB_EVENT_NAME": "schedule"}, clear=True), \
                mock.patch.object(nightly, "git", return_value=""), \
                mock.patch.object(nightly, "green_base", return_value=nightly.Ref("main", "a" * 40, None)), \
                mock.patch.object(nightly.ns, "load_state", return_value=({"prs": [{"number": 1}]}, "", None)), \
                mock.patch.object(nightly.ns, "update_state") as update, \
                mock.patch.object(nightly, "build_set") as build:
            nightly.pick(True)
            update.assert_not_called()
            self.assertIs(build.call_args.args[-1], True)
