#!/usr/bin/env python3
"""Run the admitted registered-source Game regression with an existing test binary.

The fixture loads its real pack and loaded chunks, then checks command result /
success, physical recipients, unchanged source state and clocks. It requires
prepared resources. No build or Java gameplay replay is performed by this driver.
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import hashlib
import sys
from integration_env import game_environment

TEST = 'registered_sources_reach_live_read_write_from_and_executor_context_v2'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--test-binary', required=True, type=Path)
    parser.add_argument('--resources', required=True, type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    binary = args.test_binary.resolve()
    resources = args.resources.resolve()
    assert binary.is_file() and resources.is_dir()
    assert shutil.disk_usage(root).free > 5 * 1024**3, '5 GiB engine guard'
    work = Path(tempfile.mkdtemp(prefix='horizon-registered-slot-sources-'))
    fixture = root / 'tools/fixtures/registered_slot_sources'
    env = game_environment(os.environ, work)
    # The test reads its pack and program from the repository's fixture folder.
    env.update(HORIZON_TEST_RESOURCES=str(resources))
    # A name filter that matches no test still exits 0: find the one test first, run it by its
    # exact path, and require that it ran and passed.
    listed = subprocess.run([str(binary), '--list', '--include-ignored'], capture_output=True,
                            text=True, check=True).stdout.splitlines()
    names = [line.removesuffix(': test') for line in listed
             if line.endswith('::' + TEST + ': test')]
    if len(names) != 1:
        print(f'{binary}: expected one test named {TEST}, found {names}', file=sys.stderr)
        return 1
    command = [str(binary), names[0], '--exact', '--include-ignored', '--nocapture',
               '--test-threads=1']
    receipt = {'binary': str(binary), 'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
               'command': command, 'cwd': str(work), 'fixtures': {
                   str(f.relative_to(fixture)): hashlib.sha256(f.read_bytes()).hexdigest()
                   for f in fixture.rglob('*') if f.is_file()}}
    with (work / 'test.log').open('w') as log:
        process = subprocess.Popen(command, cwd=work, env=env, stdout=log, stderr=subprocess.STDOUT)
        receipt['hard_guard_failed'] = False
        while process.poll() is None:
            if shutil.disk_usage(root).free < 5 * 1024**3:
                process.terminate()
                receipt['hard_guard_failed'] = True
                break
            try:
                process.wait(timeout=1)
            except subprocess.TimeoutExpired:
                pass
        process.wait()
    receipt['exit_code'] = process.returncode
    receipt['ran'] = 'test result: ok. 1 passed;' in (work / 'test.log').read_text()
    (work / 'receipt.json').write_text(json.dumps(receipt, indent=2))
    print(work / 'receipt.json')
    if not process.returncode and not receipt['ran']:
        print(f'{TEST} did not run and pass: see {work / "test.log"}', file=sys.stderr)
        return 1
    return process.returncode or int(receipt['hard_guard_failed'])


if __name__ == '__main__':
    sys.exit(main())
