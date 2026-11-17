"""Real-CLI contract tests for tools/compare_prepare.py with tiny synthetic preparers.

No game, Java or real preparation is involved. The two preparers are small Python scripts
supplied through the tool's public --python-tree (tar) and --binary inputs, so the tests drive
the actual CLI and read its actual report. Bytes and digests come from fixed literals and are
checked with hashlib independently of the tool under test.
"""
import hashlib
import json
import os
import stat
import subprocess
import sys
import tarfile
import tempfile
import unittest
from pathlib import Path

TOOL = Path(__file__).resolve().parents[1] / 'tools' / 'compare_prepare.py'

# Known bytes and their independently computed digests.
CANONICAL = b'PREPARED-DATA-canonical-0001'
CANONICAL_SHA = hashlib.sha256(CANONICAL).hexdigest()
NOISE_A = b'PREPARED-DATA-noise-A'
NOISE_B = b'PREPARED-DATA-noise-B'
PORT_DATA = b'PREPARED-DATA-rust-port'
CHANGED = b'PREPARED-DATA-rerun-changed'
# An unrelated file elsewhere under .local with the same basename as a prepared resource,
# but different bytes. Used to prove the integrity lookup resolves exact resource paths.
UNRELATED_SAME_BASENAME = b'PREPARED-DATA-unrelated-other-data-txt'
ZERO_SHA = '0' * 64

LABELS = ['python-in-python-1', 'python-in-python-2', 'rust-in-rust',
          'rust-in-python-2', 'python-in-rust']


def h(data):
    return 'hex:' + data.hex()


# Shared body of both fake preparers. FAKE_PREPARER is prepended by each caller. The label is
# preparer + folder, which distinguishes the cross-reruns (rust runs in python-2's folder;
# python runs in rust's folder) from the three initial runs.
FAKE_BODY = '''
import hashlib, json, os
from pathlib import Path

cwd = Path(os.getcwd())
label = FAKE_PREPARER + '-in-' + cwd.name
scenario = json.loads(os.environ['FAKE_PREPARE_SCENARIO'])
count_path = cwd / '.prep_count'
n = int(count_path.read_text()) if count_path.exists() else 0
n += 1
count_path.write_text(str(n))
out = cwd / '.local' / 'horizon-client' / '26.3'
out.mkdir(parents=True, exist_ok=True)
raw = scenario['initial'][label]
if n >= 2 and label in scenario.get('rerun', {}):
    raw = scenario['rerun'][label]
data = bytes.fromhex(raw[4:])
(out / 'data.txt').write_bytes(data)
actual = hashlib.sha256(data).hexdigest()
recorded = ZERO_SHA if label in scenario.get('stale', []) else actual
manifest = {'schema': 7, 'render_files_sha256': {'data.txt': recorded}}
if scenario.get('unrelated_same_basename'):
    other = cwd / '.local' / 'other'
    other.mkdir(parents=True, exist_ok=True)
    (other / 'data.txt').write_bytes(bytes.fromhex(scenario['unrelated_same_basename'][4:]))
if scenario.get('missing_reference'):
    manifest['render_files_sha256']['absent.txt'] = ZERO_SHA
(out / 'horizon-resources.json').write_text(json.dumps(manifest))
if n >= 2 and label in scenario.get('export', []):
    print('Exporting fixture again')
'''

PY_SOURCE = "#!/usr/bin/env python3\nFAKE_PREPARER = 'python'\nZERO_SHA = '0' * 64\n" + FAKE_BODY
RUST_SOURCE = "#!/usr/bin/env python3\nFAKE_PREPARER = 'rust'\nZERO_SHA = '0' * 64\n" + FAKE_BODY


def scenario(**overrides):
    base = {'initial': {label: h(CANONICAL) for label in LABELS}}
    initial = overrides.pop('initial', None)
    if initial:
        base['initial'].update(initial)
    base.update(overrides)
    return base


@unittest.skipIf(os.name == 'nt', 'executes directly executable shebang scripts and tar')
class ComparePrepareContractTests(unittest.TestCase):
    def setUp(self):
        # Use tempfile's default directory so an inherited TMPDIR is respected.
        self._tmp = tempfile.TemporaryDirectory(prefix='compare-prepare-contract-')
        self.addCleanup(self._tmp.cleanup)
        base = Path(self._tmp.name)
        tree = base / 'tree'
        (tree / 'tools').mkdir(parents=True)
        (tree / 'tools' / 'prepare_native_client.py').write_text(PY_SOURCE)
        (tree / 'research-lock.json').write_text(json.dumps({'minecraft': {'version': '26.3'}}))
        self.tar = base / 'python-tree.tar'
        with tarfile.open(self.tar, 'w') as archive:
            archive.add(tree / 'tools', arcname='tools')
            archive.add(tree / 'research-lock.json', arcname='research-lock.json')
        self.binary = base / 'fake-rust'
        self.binary.write_text(RUST_SOURCE)
        self.binary.chmod(self.binary.stat().st_mode | stat.S_IXUSR)
        self.jar = base / 'dummy.jar'
        self.jar.write_bytes(b'not a real jar')
        self.base = base

    def run_tool(self, config, name='work'):
        work = self.base / name
        env = dict(os.environ)
        env['FAKE_PREPARE_SCENARIO'] = json.dumps(config)
        env['PYTHONDONTWRITEBYTECODE'] = '1'
        # Inherit TMPDIR from the caller; do not pin an agent-specific path.
        proc = subprocess.run(
            [sys.executable, str(TOOL), '--server-jar', str(self.jar),
             '--minecraft-dir', str(self.base), '--python-tree', str(self.tar),
             '--binary', str(self.binary), '--work', str(work), '--keep'],
            capture_output=True, text=True, env=env, timeout=60)
        out = proc.stdout
        report = json.loads(out[out.index('{'):out.rindex('}') + 1])
        return proc, report, work

    def test_every_run_manifest_digest_must_match_its_own_file(self):
        # Positive control: all three record their own correct digest.
        proc, report, work = self.run_tool(scenario())
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertEqual(report['manifest_differences'], [])
        self.assertEqual(report['port_differences'], [])
        data = (work / 'python-2/.local/horizon-client/26.3/data.txt').read_bytes()
        self.assertEqual(data, CANONICAL)
        self.assertEqual(hashlib.sha256(data).hexdigest(), CANONICAL_SHA)

        # #17300: python-2 records a stale hash. Its own bytes and both reruns are unchanged,
        # so only per-run integrity can catch it. The rust-in-python-2 stale label keeps the
        # cross-rerun over python-2 a no-op, isolating the manifest signal.
        config = scenario(stale=['python-in-python-2', 'rust-in-python-2'])
        proc, report, work = self.run_tool(config, 'stale-python-2')
        self.assertNotEqual(proc.returncode, 0)
        self.assertTrue(any('data.txt' in field for field in report['manifest_differences']), report)
        self.assertEqual(report['rerun_resource_changes'], {'python-2': [], 'rust': []})
        self.assertEqual(report['exported_again'], {'rust over python': [], 'python over rust': []})
        py2 = work / 'python-2/.local/horizon-client/26.3'
        recorded = json.loads((py2 / 'horizon-resources.json').read_text())['render_files_sha256']['data.txt']
        self.assertEqual(recorded, ZERO_SHA)
        self.assertEqual(hashlib.sha256((py2 / 'data.txt').read_bytes()).hexdigest(), CANONICAL_SHA)

        # The first Python reference and the Rust reference are validated too.
        proc, report, _ = self.run_tool(scenario(stale=['python-in-python-1']), 'stale-python-1')
        self.assertNotEqual(proc.returncode, 0)
        self.assertTrue(report['manifest_differences'])
        proc, report, _ = self.run_tool(scenario(stale=['rust-in-rust', 'python-in-rust']), 'stale-rust')
        self.assertNotEqual(proc.returncode, 0)
        self.assertTrue(report['manifest_differences'])

        # Genuine reference noise stays allowed: bytes differ between the Python runs, but
        # each run records the digest of its own file.
        noisy = scenario(
            initial={'python-in-python-1': h(NOISE_A), 'python-in-python-2': h(NOISE_B),
                     'rust-in-rust': h(NOISE_A)},
            rerun={'rust-in-python-2': h(NOISE_B), 'python-in-rust': h(NOISE_A)})
        proc, report, _ = self.run_tool(noisy, 'noise')
        self.assertEqual(proc.returncode, 0, proc.stderr + '\n' + proc.stdout)
        self.assertIn('horizon-client/26.3/data.txt', report['exporter_noise'])
        self.assertEqual(report['manifest_differences'], [])

        # The integrity lookup must resolve the exact resource-dir-relative path, not a
        # basename. An unrelated .local/other/data.txt with different bytes must neither
        # invalidate a correct resource digest nor stand in for the prepared data.txt.
        unrelated = scenario(unrelated_same_basename=h(UNRELATED_SAME_BASENAME))
        proc, report, work = self.run_tool(unrelated, 'unrelated-basename')
        self.assertEqual(proc.returncode, 0, proc.stderr + '\n' + proc.stdout)
        self.assertEqual(report['manifest_differences'], [])
        py2_local = work / 'python-2/.local'
        self.assertEqual(hashlib.sha256((py2_local / 'horizon-client/26.3/data.txt').read_bytes()).hexdigest(),
                         CANONICAL_SHA)
        self.assertEqual((py2_local / 'other/data.txt').read_bytes(), UNRELATED_SAME_BASENAME)

        # A manifest entry naming a resource file that was never written is a failure, not a
        # lookup that can be silently skipped because no matching basename exists.
        proc, report, _ = self.run_tool(scenario(missing_reference=True), 'missing-reference')
        self.assertNotEqual(proc.returncode, 0)
        self.assertIn('render_files_sha256.absent.txt', report['manifest_differences'])
        self.assertNotIn('render_files_sha256.data.txt', report['manifest_differences'])

    def test_cross_rerun_changes_and_exports_drive_the_verdict(self):
        proc, report, _ = self.run_tool(scenario())
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertEqual(report['rerun_resource_changes'], {'python-2': [], 'rust': []})
        self.assertEqual(report['exported_again'], {'rust over python': [], 'python over rust': []})

        # #17250: a rerun that rewrites a prepared resource must fail.
        proc, report, _ = self.run_tool(scenario(rerun={'rust-in-python-2': h(CHANGED)}), 'py2-change')
        self.assertNotEqual(proc.returncode, 0)
        self.assertTrue(report['rerun_resource_changes']['python-2'])
        self.assertIn('FAILED CONDITIONS: cross-rerun resource changes', proc.stdout)
        proc, report, _ = self.run_tool(scenario(rerun={'python-in-rust': h(CHANGED)}), 'rust-change')
        self.assertNotEqual(proc.returncode, 0)
        self.assertTrue(report['rerun_resource_changes']['rust'])

        # Re-export evidence in either direction must fail. The python-over-rust log was
        # previously never inspected at all.
        proc, report, _ = self.run_tool(scenario(export=['rust-in-python-2']), 'rust-export')
        self.assertNotEqual(proc.returncode, 0)
        self.assertTrue(report['exported_again']['rust over python'])
        proc, report, _ = self.run_tool(scenario(export=['python-in-rust']), 'python-export')
        self.assertNotEqual(proc.returncode, 0)
        self.assertTrue(report['exported_again']['python over rust'])
        self.assertIn('FAILED CONDITIONS: re-export', proc.stdout)

    def test_initial_port_difference_still_fails(self):
        config = scenario(
            initial={'python-in-python-1': h(CANONICAL), 'python-in-python-2': h(CANONICAL),
                     'rust-in-rust': h(PORT_DATA)},
            rerun={'python-in-rust': h(PORT_DATA)})
        proc, report, _ = self.run_tool(config, 'port')
        self.assertNotEqual(proc.returncode, 0)
        self.assertTrue(report['port_differences'])
        self.assertIn('FAILED CONDITIONS: port differences', proc.stdout)


if __name__ == '__main__':
    unittest.main()
