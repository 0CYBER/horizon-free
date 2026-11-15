"""Exercise the workflow's shell guard without building the game or book."""
import contextlib
import io
import json
import os
from pathlib import Path
import subprocess
import tempfile
import sys
import unittest
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'tools/ci/vendor/pyyaml-6.0.3.zip'))
import yaml

WORKFLOW = ROOT / '.github/workflows/modding-book.yml'


def guard_script():
    """Read the executable guard from the parsed workflow, not text delimiters."""
    steps = yaml.safe_load(WORKFLOW.read_text())['jobs']['book']['steps']
    return next(step['run'] for step in steps
                if step.get('name') == 'Reject private paths in published docs')


def require_unreadable_file(path, credentials, cwd):
    """Probe with grep's credentials and cwd, not inaccessible home ancestors."""
    probe = subprocess.run(
        [sys.executable, '-c', '''
import json, os, stat, sys
cap = -1
if sys.platform == 'linux':
    with open('/proc/self/status') as status:
        cap = int(next(line.split()[1] for line in status if line.startswith('CapEff:')), 16)
print(json.dumps(dict(uid=os.geteuid(), gid=os.getegid(),
                     mode=stat.S_IMODE(os.stat(sys.argv[1]).st_mode), cap_eff=cap)))
try:
    with open(sys.argv[1], 'rb'):
        pass
except PermissionError:
    sys.exit(0)
sys.exit(42)
''', str(path.relative_to(cwd))], cwd=cwd, capture_output=True, text=True, **credentials)
    if probe.returncode == 42:
        # Only our fixed numeric metadata crosses this boundary, never file or
        # subprocess output text. This diagnoses the runner, not a guessed cause.
        info = json.loads(probe.stdout)
        print(f"unreadable fixture premise: uid={info['uid']:d} gid={info['gid']:d} "
              f"mode={info['mode']:03o} cap_eff={info['cap_eff']:x}")
        raise unittest.SkipTest('cannot make the file unreadable on this runner: privilege drop ineffective')
    if probe.returncode != 0:
        raise AssertionError(f'unreadable fixture probe failed: exit {probe.returncode}')


class BookGuardTests(unittest.TestCase):
    def test_parsed_workflow_keeps_scan_on_prs_and_builds_off_prs(self):
        """Pin the event gates themselves so shell tests cannot hide a skipped scan."""
        steps = yaml.safe_load(WORKFLOW.read_text())['jobs']['book']['steps']
        scan = next(step for step in steps
                    if step.get('name') == 'Reject private paths in published docs')
        present = "steps.book.outputs.present == 'true'"
        self.assertEqual(scan['if'], present)
        self.assertEqual(scan['env']['BOOK_EVENT'], '${{ github.event_name }}')
        for command in ['RUSTDOCFLAGS="-D warnings" cargo doc -p horizon-mod-api --no-deps',
                        'cargo test -p horizon-modding-examples']:
            step = next(step for step in steps if step.get('run') == command)
            self.assertEqual(step['if'], present + " && github.event_name != 'pull_request'")

    def run_guard(self, event, *, token=None, missing=None):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            paths = ['docs/modding/src', 'target/modding-book']
            if event != 'pull_request':
                paths.append('target/doc/mod_api')
            for path in paths:
                if path != missing:
                    folder = root / path
                    folder.mkdir(parents=True)
                    (folder / 'page').write_text('__private' if path == token else 'public docs')
            result = subprocess.run(
                ['bash', '-e', '-o', 'pipefail', '-c', guard_script()],
                cwd=root, env={**os.environ, 'BOOK_EVENT': event},
                capture_output=True, text=True,
            )
            return result.returncode

    def test_pr_scans_source_and_html_without_rustdoc(self):
        self.assertEqual(self.run_guard('pull_request'), 0)
        for path in ['docs/modding/src', 'target/modding-book']:
            with self.subTest(path=path):
                self.assertNotEqual(self.run_guard('pull_request', token=path), 0)
                self.assertNotEqual(self.run_guard('pull_request', missing=path), 0)

    def test_non_pr_events_require_and_scan_all_outputs(self):
        for event in ['push', 'workflow_dispatch', 'schedule']:
            self.assertEqual(self.run_guard(event), 0)
            for path in ['docs/modding/src', 'target/modding-book', 'target/doc/mod_api']:
                with self.subTest(event=event, path=path):
                    self.assertNotEqual(self.run_guard(event, token=path), 0)
                    self.assertNotEqual(self.run_guard(event, missing=path), 0)

    @unittest.skipUnless(os.name == 'posix', 'requires Unix file permissions')
    def test_unreadable_file_fails_with_real_unprivileged_grep(self):
        """Exercise a real permission error, dropping privileges when run as root."""
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            root.chmod(0o755)
            for path in ['docs/modding/src', 'target/modding-book']:
                (root / path).mkdir(parents=True)
            unreadable = root / 'docs/modding/src/unreadable.md'
            unreadable.write_text('public docs')
            unreadable.chmod(0)
            # A normal test runner is already unprivileged; root must drop to nobody.
            credentials = {'user': 65534, 'group': 65534, 'extra_groups': []} if os.geteuid() == 0 else {}
            try:
                require_unreadable_file(unreadable, credentials, root)
                result = subprocess.run(
                    ['bash', '-e', '-o', 'pipefail', '-c', guard_script()],
                    cwd=root, env={**os.environ, 'BOOK_EVENT': 'pull_request', 'LC_ALL': 'C'},
                    capture_output=True, text=True, **credentials,
                )
                self.assertNotEqual(result.returncode, 0)
                self.assertIn('Permission denied', result.stderr)
                self.assertIn('unreadable.md', result.stderr)
            finally:
                unreadable.chmod(0o600)

    @unittest.skipUnless(os.name == 'posix', 'requires Unix file permissions')
    def test_readable_fixture_causes_named_skip(self):
        """A real readable file cannot silently stand in for a permission error."""
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'readable.md'
            path.write_text('public docs')
            with contextlib.redirect_stdout(io.StringIO()) as log:
                with self.assertRaisesRegex(
                    unittest.SkipTest,
                    '^cannot make the file unreadable on this runner: privilege drop ineffective$',
                ):
                    require_unreadable_file(path, {}, path.parent)
            self.assertRegex(log.getvalue(),
                             r'^unreadable fixture premise: uid=\d+ gid=\d+ mode=[0-7]+ cap_eff=[0-9a-f-]+\n$')

    def test_broken_premise_probe_fails_instead_of_skipping(self):
        with patch.object(subprocess, 'run', return_value=subprocess.CompletedProcess([], 1, 'PRIVATE_PAYLOAD')):
            with self.assertRaisesRegex(AssertionError, '^unreadable fixture probe failed: exit 1$'):
                require_unreadable_file(Path('unused'), {}, Path('.'))

    def test_grep_errors_do_not_pass(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for path in ['docs/modding/src', 'target/modding-book']:
                (root / path).mkdir(parents=True)
            # Simulate grep's documented error status, independently of user permissions.
            bin_dir = root / 'bin'
            bin_dir.mkdir()
            grep = bin_dir / 'grep'
            grep.write_text('#!/bin/sh\nexit 2\n')
            grep.chmod(0o755)
            result = subprocess.run(
                ['bash', '-e', '-o', 'pipefail', '-c', guard_script()],
                cwd=root, env={**os.environ, 'BOOK_EVENT': 'pull_request',
                               'PATH': str(bin_dir) + os.pathsep + os.environ['PATH']},
                capture_output=True, text=True,
            )
            self.assertNotEqual(result.returncode, 0)
