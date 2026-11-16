"""Safe phase telemetry must not expose captured listing or test payloads."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


class DataLegTimingTests(unittest.TestCase):

    def test_real_cargo_boundary_markers_keep_payloads_private(self):
        with tempfile.TemporaryDirectory() as directory:
            folder = Path(directory)
            cargo = folder / "cargo"
            calls = folder / "calls.jsonl"
            cargo.write_text("""#!/usr/bin/env python3
import json, os, sys
args = sys.argv[1:]
with open(os.environ['CARGO_CALLS'], 'a') as output:
    output.write(json.dumps(args) + '\\n')
names = ['safe::case_' + str(n) for n in range(8)]
if '--list' in args:
    print('PRIVATE LISTING PAYLOAD', file=sys.stderr)
    for name in names:
        print(name + ': test')
else:
    if os.environ.get('RAW_ONLY') == '1':
        print('PRIVATE TEST PAYLOAD')
        sys.exit(0)
    skipped = {args[i + 1] for i, arg in enumerate(args) if arg == '--skip'}
    selected = [name for name in names if name not in skipped]
    print('     Running unittests src/lib.rs (target/debug/deps/example)')
    print('running ' + str(len(selected)) + ' tests')
    print('PRIVATE TEST PAYLOAD')
    for name in selected:
        print('test ' + name + ' ... ok')
    print('test result: ok. ' + str(len(selected)) + ' passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.1s')
""")
            cargo.chmod(0o755)
            result = subprocess.run([sys.executable, str(ROOT / "tools/ci/ignored_tests.py"), "example#1of2"], env={**os.environ, "PATH": directory + os.pathsep + os.environ["PATH"], "CARGO_CALLS": str(calls), "HORIZON_DATA_LEG_TIMING": "1"}, text=True, capture_output=True, check=True)
            self.assertNotIn("PRIVATE LISTING PAYLOAD", result.stdout + result.stderr)
            self.assertNotIn("PRIVATE TEST PAYLOAD", result.stdout + result.stderr)
            lines = result.stdout.splitlines()
            markers = [line for line in lines if line.startswith("data-leg phase=")]
            self.assertEqual(len(markers), 3)
            for line, phase in zip(markers, ("compile-list-start", "test-command-start", "test-command-end")):
                self.assertRegex(line, rf"^data-leg phase=example#1of2 {phase} utc=\d{{4}}-\d{{2}}-\d{{2}}T")
            self.assertRegex(markers[1], r"compile_list_seconds=\d+\.\d{3}$")
            self.assertRegex(markers[2], r"execution_seconds=\d+\.\d{3}$")
            commands = [json.loads(line) for line in calls.read_text().splitlines()]
            self.assertEqual(len(commands), 2, "telemetry must not add cargo invocations")
            self.assertIn("--list", commands[0])
            self.assertNotIn("--list", commands[1])
            self.assertIn("--include-ignored", commands[1])
            self.assertIn("--exact", commands[1])
            self.assertIn("--test-threads=4", commands[1])
            malformed = subprocess.run([sys.executable, str(ROOT / "tools/ci/ignored_tests.py"), "example"], env={**os.environ, "PATH": directory + os.pathsep + os.environ["PATH"], "CARGO_CALLS": str(calls), "RAW_ONLY": "1", "HORIZON_DATA_LEG_TIMING": "1"}, text=True, capture_output=True)
            self.assertNotEqual(malformed.returncode, 0)
            self.assertNotIn("PRIVATE TEST PAYLOAD", malformed.stdout + malformed.stderr)
            self.assertIn("raw output withheld", malformed.stdout)


if __name__ == "__main__":
    unittest.main()
