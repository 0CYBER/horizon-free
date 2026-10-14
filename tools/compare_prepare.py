#!/usr/bin/env python3
"""Check that horizon-prepare (Rust) writes what the Python preparer wrote.

Prepares the same jars three times in fresh folders made from the same tree (research-lock.json
and the vanilla exporters of --python-ref): twice with that ref's tools/prepare_native_client.py
and once with horizon-prepare. Every file under .local/ is compared by sha256:

- identical: the same bytes in all three runs;
- exporter noise: differs between the two Python runs too, so the vanilla Java exporter itself
  writes it differently each run (lambda addresses, hash-set order), not the preparer;
- port differences: identical between the Python runs but not in the Rust run. There must be none.

The manifest (horizon-resources.json) is compared field by field: only the recorded hashes of
exporter-noise files may differ, and each must be the hash of its own run's file.

Then each preparer runs again over the other's output (Rust over a Python tree, Python over the
Rust tree): the manifests are read the same way, so nothing is exported again and no resource
changes. Development tool: it needs Python 3.11+, Java and git. Report: <work>/report.json.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
# The last commit whose tools/prepare_native_client.py is the Python preparer itself.
PYTHON_REF = '66ce394d2d6dc19ed5f5c53a97b650741af9be76'
# Written by tools the comparison is not about: the decompiler only lab.py fetches, logs with
# times in them, compiled classes, and the data generator's own dated cache.
IGNORED_PARTS = {'classes', 'client-classes', 'logs', '.cache'}


def tree(ref, destination, archive=None):
    """The preparer's inputs at `ref`: research-lock.json, the exporters and the Python preparer."""
    destination.mkdir(parents=True)
    if archive is None:
        archive = subprocess.run(['git', '-C', str(ROOT), 'archive', ref, 'tools', 'mods/horizon/src/test/java',
                                  'research-lock.json'], check=True, capture_output=True).stdout
    else:
        archive = archive.read_bytes()
    subprocess.run(['tar', '-x', '-C', str(destination)], input=archive, check=True)


def hashes(folder):
    local = folder / '.local'
    result = {}
    for path in sorted(local.rglob('*')):
        relative = path.relative_to(local)
        if path.is_file() and not (set(relative.parts) & IGNORED_PARTS) and path.suffix != '.log':
            result[relative.as_posix()] = hashlib.sha256(path.read_bytes()).hexdigest()
    return result


def run(name, command, cwd, log):
    started = time.monotonic()
    with log.open('w') as stream:
        subprocess.run(command, cwd=cwd, check=True, stdout=stream, stderr=subprocess.STDOUT)
    seconds = round(time.monotonic() - started, 1)
    print(f'{name}: {seconds} s', flush=True)
    return seconds


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--server-jar', type=Path, required=True)
    parser.add_argument('--minecraft-dir', type=Path, required=True)
    parser.add_argument('--python-ref', default=PYTHON_REF)
    parser.add_argument('--python-tree', type=Path,
                        help='a tar of that tree instead of git (git archive REF tools mods/horizon/src/test/java '
                             'research-lock.json), for a machine without the repository')
    parser.add_argument('--binary', type=Path, help='horizon-prepare to test (default: cargo build --release)')
    parser.add_argument('--work', type=Path, default=ROOT / '.local/compare-prepare')
    parser.add_argument('--keep', action='store_true', help='keep the prepared folders')
    args = parser.parse_args()
    binary = args.binary
    if binary is None:
        subprocess.run(['cargo', 'build', '--release', '-p', 'horizon-prepare'], cwd=ROOT, check=True)
        metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--no-deps', '--format-version', '1'],
                                                      cwd=ROOT, text=True))
        binary = Path(metadata['target_directory']) / 'release' / (
            'horizon-prepare.exe' if os.name == 'nt' else 'horizon-prepare')
    binary = binary.resolve()
    work = args.work.resolve()
    shutil.rmtree(work, ignore_errors=True)
    work.mkdir(parents=True)
    game = ['--prepare', '--server-jar', str(args.server_jar.resolve()),
            '--minecraft-dir', str(args.minecraft_dir.resolve())]
    python = [sys.executable, '-u', 'tools/prepare_native_client.py', *game]
    folders = {name: work / name for name in ['python-1', 'python-2', 'rust']}
    times = {}
    for name, folder in folders.items():
        tree(args.python_ref, folder, args.python_tree)
        command = [str(binary), '--root', str(folder), *game] if name == 'rust' else python
        times[name] = run(name, command, folder, work / f'{name}.log')
    first, second, rust = (hashes(folders[n]) for n in ['python-1', 'python-2', 'rust'])
    names = sorted(first.keys() | second.keys() | rust.keys())
    identical = [n for n in names if first.get(n) == second.get(n) == rust.get(n) is not None]
    noise = [n for n in names if first.get(n) != second.get(n)]
    port = [n for n in names if n not in identical and n not in noise]
    # Only lab.py fetches the decompiler; preparing does not need it.
    port = [n for n in port if not (n.startswith('tools/vineflower-') and n not in rust)]
    manifest = 'horizon-client/' + json.loads((folders['rust'] / 'research-lock.json').read_text())[
        'minecraft']['version'] + '/horizon-resources.json'
    noisy = {n.rsplit('/', 1)[-1] for n in noise if n.startswith(manifest.rsplit('/', 1)[0] + '/')}
    manifests = {n: json.loads((folders[n] / '.local' / manifest).read_text()) for n in folders}
    manifest_differences = []

    def walk(python, candidate, path):
        if isinstance(python, dict) and isinstance(candidate, dict) and list(python) == list(candidate):
            for key in python:
                walk(python[key], candidate[key], path + [key])
        elif python != candidate:
            name = path[-1]
            expected = hashlib.sha256((folders['rust'] / '.local' / manifest).with_name(name).read_bytes()).hexdigest() \
                if name in noisy else None
            if not (name in noisy and candidate == expected):
                manifest_differences.append('.'.join(path))

    walk(manifests['python-1'], manifests['rust'], [])
    if manifest in port and not manifest_differences:
        port.remove(manifest)
    # Each preparer over the other's output: nothing to export again, no resource changes.
    resources = 'horizon-client/'
    before = {'python-2': second, 'rust': rust}
    times['rust over python'] = run('rust over python', [str(binary), '--root', str(folders['python-2']), *game],
                                    folders['python-2'], work / 'rust-over-python.log')
    times['python over rust'] = run('python over rust', python, folders['rust'], work / 'python-over-rust.log')
    rerun_changes = {}
    for name in before:
        after = hashes(folders[name])
        changed = [n for n in sorted(before[name].keys() | after.keys())
                   if n.startswith(resources) and before[name].get(n) != after.get(n)]
        rerun_changes[name] = changed
    exported_again = {name: [line.strip() for line in (work / log).read_text().splitlines() if 'Exporting' in line]
                      for name, log in [('rust over python', 'rust-over-python.log')]}
    report = {'python_ref': args.python_ref, 'binary': str(binary), 'seconds': times,
              'files': len(names), 'identical': len(identical), 'exporter_noise': noise,
              'port_differences': port, 'manifest_differences': manifest_differences,
              'rerun_resource_changes': rerun_changes, 'exported_again': exported_again}
    (work / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report, indent=2))
    if not args.keep:
        for folder in folders.values():
            shutil.rmtree(folder, ignore_errors=True)
    ok = not port and not manifest_differences
    print('identical output' if ok else 'DIFFERENT OUTPUT', flush=True)
    return 0 if ok else 1


if __name__ == '__main__':
    sys.exit(main())
